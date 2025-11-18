use bevy::{
    picking::mesh_picking::ray_cast::MeshRayCast,
    prelude::*,
    window::PrimaryWindow,
};

use crate::{
    camera::MainCamera,
    util,
    voxel::{voxel_raycast, ChunkData, Chunks, Coords, VoxelSizeMeters, VoxelsPerMeter},
};

#[derive(Component)]
pub struct RaycastIgnore;

#[derive(Debug, Copy, Clone)]
pub struct RayHit {
    pub entry_coords: Coords,
    pub exit_coords: Option<Coords>,
    pub hit_voxel: bool,
}

#[derive(Default, Debug, Copy, Clone)]
pub struct WorldRayHit {
    pub ray: Option<Ray3d>,
    pub ray_hit: Option<RayHit>,
}
impl WorldRayHit {
    pub fn coords(&self) -> Option<Coords> {
        self.ray_hit.as_ref().map(|hit| hit.entry_coords)
    }
}
#[derive(Resource, Debug, Default, Copy, Clone, Deref, DerefMut)]
pub struct CursorRayHit(pub WorldRayHit);
#[derive(Resource, Default, Copy, Clone, Deref, DerefMut)]
pub struct CursorRayHitWithoutDraft(pub WorldRayHit);

pub fn plugin(app: &mut App) {
    app.init_resource::<CursorRayHit>()
        .init_resource::<CursorRayHitWithoutDraft>()
        .add_systems(
            Update,
            update_world_rayhits.run_if(resource_exists::<Chunks>),
        );
}

pub fn raycast(
    raycast: &mut MeshRayCast,
    chunks: &Chunks,
    chunk_datas: &Query<&ChunkData>,
    raycast_ignores: &Query<(), With<RaycastIgnore>>,
    parents: &Query<&ChildOf>,
    voxel_size_meters: VoxelSizeMeters,
    ray: Ray3d,
    max_distance: f32,
    ignore_draft: bool,
) -> Option<RayHit> {
    let voxel_per_meters = VoxelsPerMeter::from(voxel_size_meters);

    // Cast the ray and filter out ignored entities
    let mut world_raycast: Vec<_> = raycast
        .cast_ray(ray, &MeshRayCastSettings::default())
        .iter()
        .filter(|(entity, _)| {
            util::find_parent_with_component(parents, raycast_ignores, *entity).is_none()
        })
        .collect();

    // Sort by distance to get closest hits first
    world_raycast.sort_by(|a, b| {
        let dist_a = a.1.point.distance(ray.origin);
        let dist_b = b.1.point.distance(ray.origin);
        dist_a.partial_cmp(&dist_b).unwrap()
    });

    let world_raycast_distance = world_raycast.first().map(|r| r.1.point.distance(ray.origin));

    fn world_raycast_to_hit(
        hits: &[(Entity, bevy::picking::mesh_picking::ray_cast::RayMeshHit)],
        voxels_per_meter: VoxelsPerMeter,
    ) -> Option<RayHit> {
        if hits.is_empty() {
            return None;
        }

        let entry_coords = Coords::from_world(hits[0].1.point, voxels_per_meter);
        Some(RayHit {
            entry_coords,
            exit_coords: hits
                .get(1)
                .map(|(_, h1)| Coords::from_world(h1.point, voxels_per_meter)),
            hit_voxel: false,
        })
    }

    let voxel_raycast = voxel_raycast(
        chunks,
        chunk_datas,
        voxel_size_meters,
        ray,
        max_distance,
        false,
        ignore_draft,
    );
    let voxel_raycast_distance = voxel_raycast.map(|r| {
        r.entry_coords
            .to_world(voxel_per_meters)
            .distance(ray.origin)
    });

    match (world_raycast_distance, voxel_raycast_distance) {
        (Some(wr), Some(vr)) => {
            if wr < vr {
                world_raycast_to_hit(&world_raycast, voxel_per_meters)
            } else {
                voxel_raycast
            }
        }
        (Some(_), None) => world_raycast_to_hit(&world_raycast, voxel_per_meters),
        (None, Some(_)) => voxel_raycast,
        (None, None) => None,
    }
}

pub fn update_world_rayhits(
    mut world_raycast: MeshRayCast,
    mut cursor_ray_hit: ResMut<CursorRayHit>,
    mut cursor_ray_hit_without_draft: ResMut<CursorRayHitWithoutDraft>,
    chunk_datas: Query<&ChunkData>,
    raycast_ignores: Query<(), With<RaycastIgnore>>,
    parents: Query<&ChildOf>,
    chunks: Res<Chunks>,
    voxel_size_meters: Res<VoxelSizeMeters>,
    primary_window: Query<&Window, With<PrimaryWindow>>,
    main_camera: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
) {
    let Some(cursor_position) = primary_window
        .single()
        .ok()
        .and_then(|w| w.cursor_position())
    else {
        return;
    };

    let Ok((camera, camera_transform)) = main_camera.single() else {
        return;
    };

    let ray = camera.viewport_to_world(camera_transform, cursor_position).ok();
    cursor_ray_hit.ray = ray;
    cursor_ray_hit_without_draft.ray = ray;
    if let Some(ray) = ray {
        let max_distance = 10_000.0 * voxel_size_meters.0;
        cursor_ray_hit.ray_hit = raycast(
            &mut world_raycast,
            &chunks,
            &chunk_datas,
            &raycast_ignores,
            &parents,
            *voxel_size_meters,
            ray,
            max_distance,
            false,
        );
        cursor_ray_hit_without_draft.ray_hit = raycast(
            &mut world_raycast,
            &chunks,
            &chunk_datas,
            &raycast_ignores,
            &parents,
            *voxel_size_meters,
            ray,
            max_distance,
            true,
        );
    }
}
