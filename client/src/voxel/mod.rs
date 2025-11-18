use std::sync::Arc;

use bevy::{
    prelude::*,
    utils::{HashMap, HashSet},
};

mod chunk;
use chunk::ChunkSphere;
pub use chunk::{
    ChunkBundle, ChunkCoords, ChunkData, ChunkPendingChunkSpheres, ChunkPendingDynamicUpdate,
    ChunkVisualization, Chunks,
};
use serde::{Deserialize, Serialize};

mod start_scene;
pub use start_scene::RecreateStartScene;

use crate::{color, raycast::RayHit};

/// The number of voxels in a chunk per side.
pub const VOXELS_PER_CHUNK_SIDE: usize = 25;

#[derive(Resource, Copy, Clone, Serialize, Deserialize)]
/// The number of voxels per meter. Controls all other voxel-related constants.
///
/// Must be at least 1.
#[serde(transparent)]
pub struct VoxelsPerMeter(pub usize);
/// Used to determine whether the world should be cleared (if OVPM != VPM, the world is cleared).
///
/// Set this if you want to preserve the world when changing VPM. The *only* use case for this
/// should be loading, where we are already populating the world from scratch.
#[derive(Resource, Copy, Clone)]
pub struct OldVoxelsPerMeter(pub usize);

#[derive(Resource, Copy, Clone)]
/// The size of a voxel in meters. Controlled by [`VoxelsPerMeter`].
pub struct VoxelSizeMeters(pub f32);
impl From<VoxelsPerMeter> for VoxelSizeMeters {
    fn from(voxels_per_meter: VoxelsPerMeter) -> Self {
        Self(1.0 / voxels_per_meter.0 as f32)
    }
}
impl From<VoxelSizeMeters> for VoxelsPerMeter {
    fn from(voxel_size_meters: VoxelSizeMeters) -> Self {
        Self((1.0 / voxel_size_meters.0) as usize)
    }
}

#[derive(Resource, Copy, Clone)]
/// The size of a chunk in meters. Controlled by [`VOXELS_PER_CHUNK`] and [`VoxelSizeMeters`].
pub struct ChunkSizeMeters(pub f32);
impl From<VoxelSizeMeters> for ChunkSizeMeters {
    fn from(voxel_size_meters: VoxelSizeMeters) -> Self {
        Self(VOXELS_PER_CHUNK_SIDE as f32 * voxel_size_meters.0)
    }
}

pub fn plugin(app: &mut App) {
    let initial_voxels_per_meter = VoxelsPerMeter(100);
    let initial_old_voxels_per_meter = OldVoxelsPerMeter(initial_voxels_per_meter.0);
    let initial_voxel_size_meters = VoxelSizeMeters::from(initial_voxels_per_meter);
    let initial_chunk_size_meters = ChunkSizeMeters::from(initial_voxel_size_meters);
    app.add_plugins((chunk::plugin, start_scene::plugin))
        .insert_resource(initial_voxels_per_meter)
        .insert_resource(initial_old_voxels_per_meter)
        .insert_resource(initial_voxel_size_meters)
        .insert_resource(initial_chunk_size_meters)
        .add_systems(Startup, update_sizes)
        .add_systems(PreUpdate, update_sizes);
}

// Removes all chunks in this world.
pub fn clear_all(world: &mut World) {
    world.resource_mut::<Chunks>().clear();

    let entities: Vec<_> = world
        .query_filtered::<Entity, With<ChunkData>>()
        .iter(world)
        .collect();
    for entity in entities {
        world.entity_mut(entity).despawn();
    }
    info!("Cleared all chunks");
}

fn update_sizes(
    voxels_per_meter: Res<VoxelsPerMeter>,
    mut old_voxels_per_meter: ResMut<OldVoxelsPerMeter>,
    mut voxel_size_meters: ResMut<VoxelSizeMeters>,
    mut chunk_size_meters: ResMut<ChunkSizeMeters>,
    mut commands: Commands,
) {
    if !voxels_per_meter.is_changed() {
        return;
    }

    let voxels_per_meter = voxels_per_meter.0;
    *voxel_size_meters = VoxelSizeMeters(1.0 / voxels_per_meter as f32);
    *chunk_size_meters = ChunkSizeMeters(VOXELS_PER_CHUNK_SIDE as f32 * voxel_size_meters.0);

    if voxels_per_meter != old_voxels_per_meter.0 {
        // Only clear the world if the VPM has *actually* changed
        commands.queue(|world: &mut World| {
            clear_all(world);
        });
    }
    *old_voxels_per_meter = OldVoxelsPerMeter(voxels_per_meter);
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum VoxelMaterial {
    #[default]
    Air,
    /// Not considered for raycasts and will be converted to [`VoxelMaterial::Solid`] after some time.
    /// Used to allow painting without self-interaction.
    Draft,
    Solid,
    /// This voxel was projected into the scene from a 2D image. This will not be considered
    /// when making a mask of the scene.
    Projected,
}
impl std::fmt::Display for VoxelMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Voxel {
    pub rgb: [u8; 3],
    pub material: VoxelMaterial,
}
impl Voxel {
    pub fn new(color: Color, material: VoxelMaterial) -> Self {
        Self {
            rgb: color::bevy_color_to_byte_rgb(color),
            material,
        }
    }
    pub fn with_color(mut self, color: Color) -> Self {
        self.rgb = color::bevy_color_to_byte_rgb(color);
        self
    }
    pub fn with_mixed_color(self, target_color: Color, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let self_color = color::byte_rgb_to_vec3(self.rgb);
        let target_color = color::bevy_color_to_vec3(target_color);
        self.with_color(color::vec3_to_bevy_color(self_color.lerp(target_color, t)))
    }
    pub fn with_material(mut self, material: VoxelMaterial) -> Self {
        self.material = material;
        self
    }
    /// Will convert; consider using field directly
    pub fn color(&self) -> Color {
        color::byte_rgb_to_bevy_color(self.rgb)
    }
    #[allow(dead_code)]
    pub fn as_u8_array(&self) -> [u8; 4] {
        let mut array = [0; 4];
        array[..3].copy_from_slice(&self.rgb);
        array[3] = self.material as u8;
        array
    }
    #[allow(dead_code)]
    pub fn from_u8_array(array: [u8; 4]) -> Self {
        Self {
            rgb: [array[0], array[1], array[2]],
            material: unsafe { std::mem::transmute::<u8, VoxelMaterial>(array[3]) },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// The coordinates of a voxel in the world. They are absolutely positioned in the world.
pub struct Coords(pub IVec3);
#[allow(clippy::wrong_self_convention)]
impl Coords {
    pub fn from_world(world_pos: Vec3, voxels_per_meter: VoxelsPerMeter) -> Self {
        // Do the conversion in integer math to avoid floating point precision issues
        Self(IVec3::from_array(world_pos.to_array().map(|v| {
            v.trunc() as i32 * voxels_per_meter.0 as i32
                + (v.fract() * voxels_per_meter.0 as f32) as i32
        })))
    }
    pub fn to_world(&self, voxels_per_meter: VoxelsPerMeter) -> Vec3 {
        Vec3::from_array(
            self.0
                .to_array()
                .map(|v| v as f32 / voxels_per_meter.0 as f32),
        )
    }
    pub fn chunk(&self) -> ChunkCoords {
        ChunkCoords(
            self.0
                .div_euclid(IVec3::splat(VOXELS_PER_CHUNK_SIDE as i32)),
        )
    }
    pub fn voxel(&self) -> UVec3 {
        UVec3::from_array(
            self.0
                .to_array()
                .map(|v| v.rem_euclid(VOXELS_PER_CHUNK_SIDE as i32) as u32),
        )
    }
    pub fn from_chunk_and_voxel(chunk: ChunkCoords, voxel: UVec3) -> Self {
        Self(chunk.0 * VOXELS_PER_CHUNK_SIDE as i32 + voxel.as_ivec3())
    }
    pub fn to_chunk_and_voxel(&self) -> (ChunkCoords, UVec3) {
        (self.chunk(), self.voxel())
    }
    pub fn distance_squared(&self, other: Self) -> u32 {
        self.0.distance_squared(other.0) as u32
    }
    pub fn offset(&self, offset: IVec3) -> Self {
        Self(self.0 + offset)
    }
}
impl std::fmt::Display for Coords {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Get a voxel at the given coordinates.
pub fn get(chunks: &Chunks, chunk_datas: &Query<&ChunkData>, coords: Coords) -> Option<Voxel> {
    let (chunk_coords, voxel_coords) = coords.to_chunk_and_voxel();
    let chunk = chunks.get(&chunk_coords)?;
    let chunk_data = chunk_datas.get(*chunk).ok()?;
    chunk_data.voxel(voxel_coords)
}
/// Like [`get`], but for when you have a [`World`].
pub fn get_from_world(world: &World, coords: Coords) -> Option<Voxel> {
    let (chunk_coords, voxel_coords) = coords.to_chunk_and_voxel();
    let chunk = world.resource::<Chunks>().get(&chunk_coords).copied()?;
    world.get::<ChunkData>(chunk)?.voxel(voxel_coords)
}

pub fn voxel_raycast(
    chunks: &Chunks,
    chunk_datas: &Query<&ChunkData>,
    voxel_size_meters: VoxelSizeMeters,
    ray: Ray3d,
    max_distance: f32,
    print_trace: bool,
    ignore_draft: bool,
) -> Option<RayHit> {
    let direction = ray.direction.normalize();
    let mut current_pos = ray.origin;
    let step = voxel_size_meters.0;
    let mut inside_solid = false;
    let mut entry_coords = None;

    let voxels_per_meter = VoxelsPerMeter::from(voxel_size_meters);

    for _i in 0..((max_distance / step) as i32) {
        let coords = Coords::from_world(current_pos, voxels_per_meter);

        // Check if we hit the floor (y = 0)
        if current_pos.y <= 0.0 {
            let coords = Coords::from_world(
                Vec3::new(current_pos.x, 0.0, current_pos.z),
                voxels_per_meter,
            );

            if print_trace {
                println!("{_i}: {current_pos} {coords} [ground hit]");
            }

            return Some(
                if let Some(entry_coords) = entry_coords.filter(|_| inside_solid) {
                    RayHit {
                        entry_coords,
                        exit_coords: Some(coords),
                        hit_voxel: true,
                    }
                } else {
                    RayHit {
                        entry_coords: coords,
                        exit_coords: None,
                        hit_voxel: false,
                    }
                },
            );
        }

        let is_solid = if let Some(voxel) = get(chunks, chunk_datas, coords) {
            if print_trace {
                println!("{_i}: {current_pos} {coords} [{}]", voxel.material);
            }

            voxel.material != VoxelMaterial::Air
                && (!ignore_draft || voxel.material != VoxelMaterial::Draft)
        } else {
            // A non-existent voxel can be treated as air
            false
        };

        if is_solid && !inside_solid {
            // Entering solid voxel
            inside_solid = true;
            entry_coords = Some(coords);
        } else if !is_solid && inside_solid {
            // Exiting solid voxel
            return Some(RayHit {
                entry_coords: entry_coords.unwrap(),
                exit_coords: Some(coords),
                hit_voxel: true,
            });
        }

        // Move to next position
        current_pos += direction * step;
    }

    // If we're still inside a solid voxel when reaching max_distance
    if inside_solid {
        return Some(RayHit {
            entry_coords: entry_coords.unwrap(),
            exit_coords: None,
            hit_voxel: true,
        });
    }

    // No hit within max_distance
    None
}

#[allow(dead_code)]
pub struct Splat {
    pub position: Vec3,
    pub color: Vec3,
    pub size: f32,
}

/// Bulk-inserts splats into the world.
pub fn bulk_insert_splats(
    chunks: &mut Chunks,
    pending_chunk_spheres: &mut Query<&mut ChunkPendingChunkSpheres>,
    voxels_per_meter: VoxelsPerMeter,
    commands: &mut Commands,
    splats: &[Splat],
) {
    // Group spheres by chunk
    let now = web_time::Instant::now();
    let mut chunk_spheres: HashMap<ChunkCoords, Vec<ChunkSphere>> = HashMap::default();
    let mut updated_chunks = HashSet::default();
    for instance in splats {
        let center = Coords::from_world(instance.position, voxels_per_meter);
        let radius_voxels = (instance.size * voxels_per_meter.0 as f32) as u32;

        fn add_sphere(
            updated_chunks: &mut HashSet<ChunkCoords>,
            chunk_spheres: &mut HashMap<ChunkCoords, Vec<ChunkSphere>>,
            coords_for_chunk: Coords,
            chunk_sphere: ChunkSphere,
        ) {
            let chunk_coords = coords_for_chunk.chunk();
            if chunk_coords.y < 0 {
                return;
            }
            if updated_chunks.contains(&chunk_coords) {
                return;
            }
            chunk_spheres
                .entry(chunk_coords)
                .or_default()
                .push(chunk_sphere);
            updated_chunks.insert(chunk_coords);
        }

        let sphere = ChunkSphere {
            center,
            radius_voxels,
            radius_voxels_sqr: radius_voxels * radius_voxels,
            color: instance.color.to_array().map(|v| (v * 255.0) as u8),
        };

        add_sphere(&mut updated_chunks, &mut chunk_spheres, center, sphere);
        // Add surrounding chunks that the sphere overlaps with
        let r = radius_voxels as i32;
        for z in [-r, r] {
            for y in [-r, r] {
                for x in [-r, r] {
                    let new_coords = center.offset(IVec3::new(x, y, z));
                    add_sphere(&mut updated_chunks, &mut chunk_spheres, new_coords, sphere);
                }
            }
        }
        updated_chunks.clear();
    }
    info!("Grouping spheres by chunk took {:?}", now.elapsed());

    let now = web_time::Instant::now();
    for (coords, mut spheres) in chunk_spheres {
        if let Some(chunk) = chunks.get(&coords) {
            let mut pending_chunk_spheres = pending_chunk_spheres.get_mut(*chunk).unwrap();
            pending_chunk_spheres.0.append(&mut spheres);
        } else {
            let chunk_id = commands.spawn(ChunkBundle::new(coords, None, spheres)).id();
            chunks.insert(coords, chunk_id);
        }
    }
    info!("Updating chunks took {:?}", now.elapsed());
}

pub struct UpdateParams<
    AllocateChunkFn: Fn(Coords, f32) -> bool + Send + Sync + Copy + 'static,
    UpdateFn: Fn(Coords, Voxel) -> Voxel + Send + Sync + Copy + 'static,
> {
    /// The center of where the update is happening.
    pub center: Coords,
    /// The distance in meters from the center to update.
    pub distance_meters: f32,
    /// This function will be called with the centre of each chunk. If it returns true,
    /// a new chunk will be allocated if necessary.
    ///
    /// If your update is well-bounded by `distance_meters`, you can use `|_| true`.
    /// If your update will never allocate new chunks, you can use `|_| false`.
    /// If your update is more complex (i.e. non-symmetrical), consider implementing this
    /// with a SDF or similar.
    pub should_allocate_chunk: AllocateChunkFn,
    /// The function with which to update the voxel.
    pub update: UpdateFn,
}

/// Updates the voxel world with the given function. The function is called for each voxel within
/// `distance_meters` of `center`. The function should return the new voxel value.
///
/// Updates are deferred until the next frame.
pub fn update<
    AllocateChunkFn: Fn(Coords, f32) -> bool + Send + Sync + Copy + 'static,
    UpdateFn: Fn(Coords, Voxel) -> Voxel + Send + Sync + Copy + 'static,
>(
    pending_dynamic_updates: &mut EventWriter<ChunkPendingDynamicUpdate>,
    // TODO: deal with the potential bug if VPM changes between an update being issued and it being processed
    //
    // The logic here will allocate updates based on the current VPM/chunk size, but if the VPM changes
    // that will all be invalidated.
    voxels_per_meter: VoxelsPerMeter,
    params: UpdateParams<AllocateChunkFn, UpdateFn>,
) {
    let UpdateParams {
        center,
        distance_meters,
        should_allocate_chunk,
        update,
    } = params;

    let distance_voxels = (distance_meters * voxels_per_meter.0 as f32) as i32;
    let min_coords = Coords(center.0 - IVec3::splat(distance_voxels));
    let max_coords = Coords(center.0 + IVec3::splat(distance_voxels));

    let should_allocate_chunk = Arc::new(should_allocate_chunk);
    let (min_chunk, max_chunk) = (min_coords.chunk(), max_coords.chunk());
    for chunk_x in min_chunk.x..=max_chunk.x {
        for chunk_y in min_chunk.y..=max_chunk.y {
            if chunk_y < 0 {
                continue;
            }

            for chunk_z in min_chunk.z..=max_chunk.z {
                let coords = ChunkCoords(IVec3::new(chunk_x, chunk_y, chunk_z));
                pending_dynamic_updates.send(ChunkPendingDynamicUpdate {
                    coords,
                    should_allocate_chunk: should_allocate_chunk.clone(),
                    operation: Arc::new(move |chunk_data, chunk_coords| {
                        let mut has_draft_material = false;
                        for z in 0..VOXELS_PER_CHUNK_SIDE {
                            for y in 0..VOXELS_PER_CHUNK_SIDE {
                                for x in 0..VOXELS_PER_CHUNK_SIDE {
                                    let voxel_coords = UVec3::new(x as u32, y as u32, z as u32);
                                    let coords =
                                        Coords::from_chunk_and_voxel(chunk_coords, voxel_coords);

                                    let diff = coords.0 - center.0;
                                    if diff.x.abs() <= distance_voxels
                                        && diff.y.abs() <= distance_voxels
                                        && diff.z.abs() <= distance_voxels
                                    {
                                        let voxel = chunk_data.voxel_mut(voxel_coords).unwrap();
                                        let new_voxel = update(coords, *voxel);
                                        if *voxel != new_voxel {
                                            *voxel = new_voxel;
                                            if voxel.material == VoxelMaterial::Draft {
                                                has_draft_material = true;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        has_draft_material
                    }),
                });
            }
        }
    }
}
