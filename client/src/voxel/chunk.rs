//! Manages the voxel chunks in the game world.
//!
//! Note that the majority of voxel updates are done through the [`ChunkPendingDynamicUpdate`] event
//! or a similar mechanism. This allows for the updates to be done in a batch, which is more
//! efficient than updating each voxel individually. It also prevents issues that arise from
//! chunks not existing prior to the update.

use std::{
    collections::HashMap,
    sync::{atomic::AtomicBool, Arc, Mutex},
};

use avian3d::prelude::*;
use bevy::{
    asset::{embedded_asset, RenderAssetUsages},
    camera::visibility::RenderLayers,
    mesh::{Indices, PrimitiveTopology},
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
};
use bevy_egui::{egui, EguiContexts};

use crate::{
    camera::MainCamera,
    raycast::RaycastIgnore,
    rendering::{AlphaPulse, MainCameraGizmos, MASK_CAMERA_ONLY_LAYER},
    AppState,
};

use super::{
    ChunkSizeMeters, Coords, Voxel, VoxelMaterial, VoxelSizeMeters, VOXELS_PER_CHUNK_SIDE,
};

pub fn plugin(app: &mut App) {
    app.init_resource::<Chunks>()
        .init_resource::<ChunkVisualization>()
        .add_plugins(MaterialPlugin::<
            ExtendedMaterial<StandardMaterial, MatteMaterialExtension>,
        >::default())
        .add_message::<ChunkPendingDynamicUpdate>()
        .add_systems(Startup, setup)
        .add_systems(PreUpdate, build_chunk_map)
        .add_systems(
            Update,
            (
                ensure_chunks_exist_for_updates,
                extract_updates_into_chunks,
                apply_splats,
                apply_chunk_updates,
                promote_draft_voxels_to_real_voxels,
                switch_materials_when_draft_chunk,
                switch_materials_when_no_longer_draft_chunk,
                garbage_collect_chunks,
                rebuild_updated_chunks,
                visualize_chunks,
            )
                .chain(),
        )
        .add_systems(OnEnter(AppState::Play), add_physics_to_chunks);
    embedded_asset!(app, "matte_material.wgsl");
}

#[derive(Resource, Debug, Deref, DerefMut, Default)]
pub struct Chunks(pub HashMap<ChunkCoords, Entity>);

#[derive(Resource, Default)]
pub struct ChunkVisualization(pub bool);

#[derive(Component, Debug, Deref, DerefMut, PartialEq, Copy, Clone, Hash, Eq)]
pub struct ChunkCoords(pub IVec3);
impl ChunkCoords {
    #[allow(clippy::wrong_self_convention)]
    pub fn to_world(&self, chunk_size_meters: ChunkSizeMeters) -> Vec3 {
        Vec3::from(self.0.to_array().map(|v| v as f32 * chunk_size_meters.0))
    }
    pub fn with_voxel_coords(&self, voxel_coords: UVec3) -> Coords {
        Coords::from_chunk_and_voxel(*self, voxel_coords)
    }
}
impl std::fmt::Display for ChunkCoords {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, PartialEq, Eq, Component)]
pub struct ChunkData {
    pub voxels: Vec<Voxel>,
}
impl Default for ChunkData {
    fn default() -> Self {
        Self {
            voxels: vec![
                Voxel::default();
                VOXELS_PER_CHUNK_SIDE * VOXELS_PER_CHUNK_SIDE * VOXELS_PER_CHUNK_SIDE
            ],
        }
    }
}
impl ChunkData {
    /// Scans through all the voxels in the chunk to check if it is completely empty.
    pub fn is_completely_empty(&self) -> bool {
        self.voxels.iter().all(|v| v.material == VoxelMaterial::Air)
    }
    /// Scans through all the voxels in the chunk to check if it is completely solid.
    pub fn is_completely_solid(&self) -> bool {
        self.voxels.iter().all(|v| v.material != VoxelMaterial::Air)
    }
    pub fn voxel(&self, coords: UVec3) -> Option<Voxel> {
        self.voxels.get(voxel_coord_to_index(coords)?).copied()
    }
    pub fn voxel_mut(&mut self, coords: UVec3) -> Option<&mut Voxel> {
        self.voxels.get_mut(voxel_coord_to_index(coords)?)
    }
    #[allow(dead_code)]
    pub fn iter(&self) -> impl Iterator<Item = &Voxel> {
        self.voxels.iter()
    }
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Voxel> {
        self.voxels.iter_mut()
    }
}
fn voxel_coord_to_index(coords: UVec3) -> Option<usize> {
    let x: usize = coords.x.try_into().ok()?;
    let y: usize = coords.y.try_into().ok()?;
    let z: usize = coords.z.try_into().ok()?;
    if x >= VOXELS_PER_CHUNK_SIDE || y >= VOXELS_PER_CHUNK_SIDE || z >= VOXELS_PER_CHUNK_SIDE {
        return None;
    }
    Some(x + (y * VOXELS_PER_CHUNK_SIDE) + (z * VOXELS_PER_CHUNK_SIDE * VOXELS_PER_CHUNK_SIDE))
}

#[derive(Component, PartialEq, Clone)]
pub struct ChunkHasDraftVoxels;

/// A sphere of voxels that will be applied to the world.
///
/// Used by [`super::bulk_insert_splats`].
#[derive(Debug, Clone, Copy)]
pub struct ChunkSphere {
    pub center: Coords,
    pub radius_voxels: u32,
    pub radius_voxels_sqr: u32,
    pub color: [u8; 3],
}

#[derive(Component, Default)]
pub struct ChunkPendingChunkSpheres(pub Vec<ChunkSphere>);

#[derive(Bundle)]
pub struct ChunkBundle {
    pub name: Name,
    pub data: ChunkData,
    pub coords: ChunkCoords,
    pub last_updated: ChunkLastUpdated,
    pub pending_spheres: ChunkPendingChunkSpheres,
    pub pending_dynamic_updates: ChunkPendingDynamicUpdates,
    pub raycast_ignore: RaycastIgnore,
}
impl ChunkBundle {
    pub fn new(
        coords: ChunkCoords,
        data: Option<ChunkData>,
        pending_spheres: Vec<ChunkSphere>,
    ) -> Self {
        Self {
            name: Name::new(format!("Chunk {coords}")),
            data: data.unwrap_or_default(),
            coords,
            last_updated: ChunkLastUpdated::now(),
            pending_spheres: ChunkPendingChunkSpheres(pending_spheres),
            pending_dynamic_updates: ChunkPendingDynamicUpdates::default(),
            raycast_ignore: RaycastIgnore,
        }
    }
}

#[derive(Component)]
pub struct ChunkLastUpdated(pub web_time::Instant);
impl ChunkLastUpdated {
    pub fn now() -> Self {
        Self(web_time::Instant::now())
    }
}

/// Return type is whether the operation has created draft voxels or not
#[allow(clippy::type_complexity)]
pub type ChunkDynamicUpdate = Arc<dyn Fn(&mut ChunkData, ChunkCoords) -> bool + Send + Sync>;

#[derive(Message)]
/// When sent, the associated chunk will be updated with the given function.
/// If the chunk does not exist, it will be created before the update is applied.
pub struct ChunkPendingDynamicUpdate {
    pub coords: ChunkCoords,
    #[allow(clippy::type_complexity)]
    pub should_allocate_chunk: Arc<dyn Fn(Coords, f32) -> bool + Send + Sync + 'static>,
    pub operation: ChunkDynamicUpdate,
}

#[derive(Component, Default)]
pub struct ChunkPendingDynamicUpdates(Vec<ChunkDynamicUpdate>);

#[derive(Resource, Deref)]
struct ChunkMaterial(Handle<StandardMaterial>);

#[derive(Resource, Deref)]
struct ChunkDraftMaterial(Handle<StandardMaterial>);

#[derive(Resource, Deref)]
struct ChunkMaskMaterial(Handle<ExtendedMaterial<StandardMaterial, MatteMaterialExtension>>);

const GARBAGE_COLLECION_TIME: web_time::Duration = web_time::Duration::from_secs(10);
const DRAFT_CONVERSION_TIME: web_time::Duration = web_time::Duration::from_secs(1);

fn setup(
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut extended_materials: ResMut<
        Assets<ExtendedMaterial<StandardMaterial, MatteMaterialExtension>>,
    >,
    mut commands: Commands,
) {
    let desc = StandardMaterial {
        reflectance: 0.0,
        unlit: true,
        depth_bias: 0.2,
        ..default()
    };
    let chunk_material = materials.add(desc.clone());
    commands.insert_resource(ChunkMaterial(chunk_material));

    let draft_material = materials.add(StandardMaterial {
        alpha_mode: AlphaMode::Blend,
        ..desc.clone()
    });
    commands.insert_resource(ChunkDraftMaterial(draft_material));

    let mask_material = extended_materials.add(ExtendedMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            ..desc
        },
        extension: MatteMaterialExtension {},
    });
    commands.insert_resource(ChunkMaskMaterial(mask_material));
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
struct MatteMaterialExtension {}
impl MaterialExtension for MatteMaterialExtension {
    fn fragment_shader() -> ShaderRef {
        "embedded://prismata_client/voxel/matte_material.wgsl".into()
    }

    fn deferred_fragment_shader() -> ShaderRef {
        "embedded://prismata_client/voxel/matte_material.wgsl".into()
    }
}

fn build_chunk_map(mut chunks: ResMut<Chunks>, chunk_data: Query<(&ChunkCoords, Entity)>) {
    chunks.0 = HashMap::from_iter(chunk_data.iter().map(|(coords, entity)| (*coords, entity)));
}

fn ensure_chunks_exist_for_updates(
    mut pending_dynamic_updates: MessageReader<ChunkPendingDynamicUpdate>,
    mut all_chunks: ResMut<Chunks>,
    chunk_size_meters: Res<ChunkSizeMeters>,
    mut chunks: Query<&mut ChunkLastUpdated>,
    mut commands: Commands,
) {
    for update in pending_dynamic_updates.read() {
        if let Some(id) = all_chunks.0.get(&update.coords) {
            // Ensure that any chunk we're about to update won't be GC'd
            if let Ok(mut chunk_last_updated) = chunks.get_mut(*id) {
                // We check before doing this update as this very system may have allocated
                // an entity that we're about to update, and that entity won't be visible
                // until the next tick
                *chunk_last_updated = ChunkLastUpdated::now();
            }
        } else if (update.should_allocate_chunk)(
            update
                .coords
                .with_voxel_coords(UVec3::splat(VOXELS_PER_CHUNK_SIDE as u32 / 2)),
            chunk_size_meters.0,
        ) {
            let id = commands
                .spawn(ChunkBundle::new(update.coords, None, vec![]))
                .id();
            all_chunks.0.insert(update.coords, id);
        }
    }
}

fn extract_updates_into_chunks(
    mut pending_dynamic_updates: MessageReader<ChunkPendingDynamicUpdate>,
    mut chunks: Query<&mut ChunkPendingDynamicUpdates>,
    all_chunks: Res<Chunks>,
) {
    for update in pending_dynamic_updates.read() {
        if let Some(chunk_id) = all_chunks.0.get(&update.coords).copied() {
            if let Ok(mut pending_updates) = chunks.get_mut(chunk_id) {
                pending_updates.0.push(update.operation.clone());
            }
        }
    }
}

fn apply_splats(
    mut chunks: Query<(
        &ChunkCoords,
        &mut ChunkData,
        &mut ChunkPendingChunkSpheres,
        &mut ChunkLastUpdated,
    )>,
) {
    let now = web_time::Instant::now();
    let processed_any = AtomicBool::new(false);
    chunks.par_iter_mut().for_each(
        |(chunk_coords, mut chunk_data, mut pending_spheres, mut chunk_last_updated)| {
            if pending_spheres.0.is_empty() {
                return;
            }

            processed_any.store(true, std::sync::atomic::Ordering::Relaxed);

            for sphere in pending_spheres.0.drain(..) {
                // This is obviously wrong and should be diameter. Fix later.
                let half_radius_voxels = sphere.radius_voxels as i32 / 2;
                for z in -half_radius_voxels..half_radius_voxels {
                    for y in -half_radius_voxels..half_radius_voxels {
                        for x in -half_radius_voxels..half_radius_voxels {
                            let coords = sphere.center.offset(IVec3::new(x, y, z));
                            let (this_chunk_coords, this_voxel_coords) =
                                coords.to_chunk_and_voxel();
                            if this_chunk_coords != *chunk_coords {
                                continue;
                            }
                            if coords.distance_squared(sphere.center) > sphere.radius_voxels_sqr {
                                continue;
                            }

                            let voxel = chunk_data.voxel_mut(this_voxel_coords).unwrap();
                            let final_color = if voxel.material == VoxelMaterial::Air {
                                sphere.color
                            } else {
                                let old_color = voxel.rgb.map(|c| c as u16);
                                let new_color = sphere.color.map(|c| c as u16);
                                [
                                    (old_color[0] + new_color[0]) / 2,
                                    (old_color[1] + new_color[1]) / 2,
                                    (old_color[2] + new_color[2]) / 2,
                                ]
                                .map(|c| c as u8)
                            };
                            *voxel = Voxel {
                                rgb: final_color,
                                material: VoxelMaterial::Projected,
                            };
                        }
                    }
                }
            }

            *chunk_last_updated = ChunkLastUpdated::now();
        },
    );
    if processed_any.load(std::sync::atomic::Ordering::Relaxed) {
        info!("Applying splats took {:?}", now.elapsed());
    }
}

fn apply_chunk_updates(
    mut chunks: Query<(
        Entity,
        &ChunkCoords,
        &mut ChunkData,
        &mut ChunkLastUpdated,
        &mut ChunkPendingDynamicUpdates,
    )>,
    mut commands: Commands,
) {
    // We could use ParallelCommands here, but it seems likely to be faster to just use a mutex
    let draft_chunks = Mutex::new(vec![]);
    chunks.par_iter_mut().for_each(
        |(
            chunk_id,
            chunk_coords,
            mut chunk_data,
            mut chunk_last_updated,
            mut pending_dynamic_updates,
        )| {
            let mut did_update = false;
            for update in pending_dynamic_updates.0.drain(..) {
                let has_draft_material = (update)(&mut chunk_data, *chunk_coords);
                if has_draft_material {
                    draft_chunks.lock().unwrap().push(chunk_id);
                }
                did_update = true;
            }
            if did_update {
                *chunk_last_updated = ChunkLastUpdated::now();
            }
        },
    );
    for chunk_id in draft_chunks.into_inner().unwrap() {
        commands.entity(chunk_id).insert(ChunkHasDraftVoxels);
    }
}

fn promote_draft_voxels_to_real_voxels(
    mut chunks: Query<(Entity, &mut ChunkData, &ChunkLastUpdated), With<ChunkHasDraftVoxels>>,
    mut commands: Commands,
) {
    for (entity, mut chunk_data, chunk_last_updated) in chunks.iter_mut() {
        if chunk_last_updated.0.elapsed() < DRAFT_CONVERSION_TIME {
            return;
        }

        for voxel in chunk_data.iter_mut() {
            if voxel.material == VoxelMaterial::Draft {
                voxel.material = VoxelMaterial::Solid;
            }
        }
        commands.entity(entity).remove::<ChunkHasDraftVoxels>();
    }
}

fn switch_materials_when_draft_chunk(
    draft_material: Res<ChunkDraftMaterial>,
    mut chunks: Query<(Entity, &mut MeshMaterial3d<StandardMaterial>), Added<ChunkHasDraftVoxels>>,
    mut commands: Commands,
) {
    for (entity, mut material) in chunks.iter_mut() {
        material.0 = draft_material.0.clone();
        commands.entity(entity).insert(AlphaPulse::new(0.2, 0.5));
    }
}

fn switch_materials_when_no_longer_draft_chunk(
    standard_material: Res<ChunkMaterial>,
    mut removed_draft_chunks: RemovedComponents<ChunkHasDraftVoxels>,
    mut chunks: Query<(Entity, &mut MeshMaterial3d<StandardMaterial>)>,
    mut commands: Commands,
) {
    for id in removed_draft_chunks.read() {
        if let Ok((entity, mut material)) = chunks.get_mut(id) {
            material.0 = standard_material.0.clone();
            commands.entity(entity).remove::<AlphaPulse>();
        }
    }
}

fn garbage_collect_chunks(
    mut all_chunks: ResMut<Chunks>,
    chunks: Query<(&ChunkData, &ChunkLastUpdated)>,
    mut commands: Commands,
) {
    let old_chunk_count = all_chunks.0.len();
    let mut chunks_to_remove = Vec::new();
    for (chunk_coords, chunk_id) in &all_chunks.0 {
        let (chunk_data, chunk_last_updated) = chunks.get(*chunk_id).unwrap();
        if chunk_last_updated.0.elapsed() > GARBAGE_COLLECION_TIME
            && chunk_data.is_completely_empty()
        {
            chunks_to_remove.push(*chunk_coords);
        }
    }
    for chunk_coords in chunks_to_remove {
        if let Some(chunk_entity) = all_chunks.remove(&chunk_coords) {
            commands.entity(chunk_entity).despawn();
        }
    }
    let new_chunk_count = all_chunks.0.len();
    let delta = old_chunk_count - new_chunk_count;
    if delta > 0 {
        info!("Garbage collected {} chunks", delta);
    }
}

fn rebuild_updated_chunks(
    mut meshes: ResMut<Assets<Mesh>>,
    mut updated_chunks: Query<
        (Entity, &ChunkData, &ChunkCoords, Option<&mut Mesh3d>),
        Changed<ChunkData>,
    >,
    chunk_material: Res<ChunkMaterial>,
    chunk_mask_material: Res<ChunkMaskMaterial>,
    chunk_size_meters: Res<ChunkSizeMeters>,
    voxel_size_meters: Res<VoxelSizeMeters>,
    mut commands: Commands,
) {
    for (chunk_id, chunk_data, chunk_coords, mesh) in updated_chunks.iter_mut() {
        let handle = meshes.add(build_mesh(chunk_data, false));
        if let Some(mut mesh) = mesh {
            mesh.0 = handle;
        } else {
            let translation = chunk_coords.to_world(*chunk_size_meters);
            commands.entity(chunk_id).insert((
                Mesh3d(handle),
                MeshMaterial3d(chunk_material.0.clone()),
                Transform::from_translation(translation)
                    .with_scale(Vec3::splat(voxel_size_meters.0)),
            ));
        }

        commands
            .entity(chunk_id)
            .despawn_related::<Children>()
            .with_children(|b| {
                let handle = meshes.add(build_mesh(chunk_data, true));
                b.spawn((
                    Mesh3d(handle),
                    MeshMaterial3d(chunk_mask_material.clone()),
                    Transform::default(),
                    RenderLayers::layer(MASK_CAMERA_ONLY_LAYER as usize),
                ));
            });
    }
}

pub fn build_mesh(data: &ChunkData, for_mask_mesh: bool) -> Mesh {
    let mut vertices = vec![];
    let mut indices = vec![];
    let mut colors = vec![];
    let mut index_offset = 0;

    for x in 0..VOXELS_PER_CHUNK_SIDE {
        for y in 0..VOXELS_PER_CHUNK_SIDE {
            for z in 0..VOXELS_PER_CHUNK_SIDE {
                let voxel = data
                    .voxel(UVec3::new(x as u32, y as u32, z as u32))
                    .unwrap();
                if voxel.material == VoxelMaterial::Air {
                    continue;
                }
                let pos = Vec3::new(x as f32, y as f32, z as f32);
                let color = if for_mask_mesh {
                    if voxel.material == VoxelMaterial::Projected {
                        Vec3::splat(0.0)
                    } else {
                        Vec3::splat(1.0)
                    }
                } else {
                    Vec3::new(
                        voxel.rgb[0] as f32 / 255.0,
                        voxel.rgb[1] as f32 / 255.0,
                        voxel.rgb[2] as f32 / 255.0,
                    )
                };
                add_visible_faces(
                    data,
                    x,
                    y,
                    z,
                    &mut vertices,
                    &mut indices,
                    &mut index_offset,
                    &mut colors,
                    pos,
                    color,
                );
            }
        }
    }

    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vertices)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    // TODO: normals?
    .with_inserted_indices(Indices::U16(indices))
}

fn add_visible_faces(
    data: &ChunkData,
    x: usize,
    y: usize,
    z: usize,
    vertices: &mut Vec<[f32; 3]>,
    indices: &mut Vec<u16>,
    index_offset: &mut u16,
    colors: &mut Vec<[f32; 4]>,
    position: Vec3,
    color: Vec3,
) {
    let faces = [
        // Front
        (
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            0,
            0,
            1,
        ),
        // Back
        (
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            0,
            0,
            -1,
        ),
        // Left
        (
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(0.0, 1.0, 0.0),
            -1,
            0,
            0,
        ),
        // Right
        (
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            1,
            0,
            0,
        ),
        // Top (mirrored bottom face)
        (
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            0,
            1,
            0,
        ),
        // Bottom
        (
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            0,
            -1,
            0,
        ),
    ];

    let color = color.extend(1.0).to_array();
    for (origin, du, dv, dx, dy, dz) in faces.iter() {
        if is_face_visible(data, x, y, z, *dx, *dy, *dz) {
            vertices.extend_from_slice(&[
                (position + *origin).to_array(),
                (position + *origin + *du).to_array(),
                (position + *origin + *du + *dv).to_array(),
                (position + *origin + *dv).to_array(),
            ]);
            colors.extend_from_slice(&[color, color, color, color]);

            // Adjust the winding order for the top face
            if *dy == 1 {
                indices.extend_from_slice(&[
                    *index_offset,
                    *index_offset + 2,
                    *index_offset + 1,
                    *index_offset + 2,
                    *index_offset,
                    *index_offset + 3,
                ]);
            } else {
                indices.extend_from_slice(&[
                    *index_offset,
                    *index_offset + 1,
                    *index_offset + 2,
                    *index_offset + 2,
                    *index_offset + 3,
                    *index_offset,
                ]);
            }

            *index_offset += 4;
        }
    }
}

fn is_face_visible(
    chunk: &ChunkData,
    x: usize,
    y: usize,
    z: usize,
    dx: isize,
    dy: isize,
    dz: isize,
) -> bool {
    let nx = x as isize + dx;
    let ny = y as isize + dy;
    let nz = z as isize + dz;

    if nx < 0
        || ny < 0
        || nz < 0
        || nx >= VOXELS_PER_CHUNK_SIDE as isize
        || ny >= VOXELS_PER_CHUNK_SIDE as isize
        || nz >= VOXELS_PER_CHUNK_SIDE as isize
    {
        return true; // Faces at chunk borders are always visible
    }

    chunk
        .voxel(UVec3::new(nx as u32, ny as u32, nz as u32))
        .unwrap()
        .material
        == VoxelMaterial::Air
}

fn visualize_chunks(
    mut egui_contexts: EguiContexts,
    mut gizmos: Gizmos<MainCameraGizmos>,
    chunk_visualization: Res<ChunkVisualization>,
    chunk_size_meters: Res<ChunkSizeMeters>,
    our_camera: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    chunks: Query<(&Transform, &ChunkCoords, Has<ChunkHasDraftVoxels>)>,
) {
    if !chunk_visualization.0 {
        return;
    }
    let Ok((our_camera, our_camera_transform)) = our_camera.single() else {
        return;
    };

    let egui_context = egui_contexts.ctx_mut().unwrap();
    let font = egui::TextStyle::Monospace.resolve(&egui_context.style());

    for (transform, coords, has_draft_voxels) in chunks.iter() {
        let position = transform.translation + Vec3::splat(chunk_size_meters.0 / 2.0);
        gizmos.primitive_3d(
            &Cuboid::from_size(Vec3::splat(chunk_size_meters.0)),
            Isometry3d::new(position, transform.rotation),
            if has_draft_voxels {
                Color::linear_rgb(1.0, 1.0, 0.0)
            } else {
                Color::linear_rgb(0.0, 1.0, 0.0)
            },
        );

        let Ok(screen_pos) = our_camera.world_to_viewport(our_camera_transform, position) else {
            continue;
        };
        egui_context.debug_painter().text(
            screen_pos.to_array().into(),
            egui::Align2::CENTER_CENTER,
            coords.to_string(),
            font.clone(),
            egui::Color32::WHITE,
        );
    }
}

fn add_physics_to_chunks(mut commands: Commands, chunks: Query<(Entity, &ChunkData)>) {
    for (entity, chunk_data) in chunks.iter() {
        if chunk_data.is_completely_empty() {
            continue;
        }
        let is_completely_solid = chunk_data.is_completely_solid();
        commands.entity(entity).insert((
            RigidBody::Static,
            if is_completely_solid {
                ColliderConstructor::Cuboid {
                    x_length: 2.0,
                    y_length: 2.0,
                    z_length: 2.0,
                }
            } else {
                ColliderConstructor::ConvexHullFromMesh
            },
        ));
    }
}
