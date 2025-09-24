use super::*;

#[derive(Event)]
pub struct RecreateStartScene;

pub fn plugin(app: &mut App) {
    app.add_event::<RecreateStartScene>()
        .add_systems(Update, recreate_start_scene_on_event);
}

fn create_start_scene(
    mut pending_dynamic_updates: EventWriter<ChunkPendingDynamicUpdate>,
    voxels_per_meter: Res<VoxelsPerMeter>,
) {
    let pending_dynamic_updates = &mut pending_dynamic_updates;
    let voxels_per_meter = *voxels_per_meter;

    original_sphere(pending_dynamic_updates, voxels_per_meter);
    perlinlike_noise_sphere(pending_dynamic_updates, voxels_per_meter);
    torus(pending_dynamic_updates, voxels_per_meter);
    fractal_terrain(pending_dynamic_updates, voxels_per_meter);
    spiral_staircase(pending_dynamic_updates, voxels_per_meter);

    info!("Created start scene");
}

fn recreate_start_scene_on_event(
    mut recreate_start_scene_events: EventReader<RecreateStartScene>,
    pending_dynamic_updates: EventWriter<ChunkPendingDynamicUpdate>,
    voxels_per_meter: Res<VoxelsPerMeter>,
) {
    if recreate_start_scene_events.read().count() > 0 {
        create_start_scene(pending_dynamic_updates, voxels_per_meter);
    }
}

fn original_sphere(
    pending_dynamic_updates: &mut EventWriter<ChunkPendingDynamicUpdate>,
    voxels_per_meter: VoxelsPerMeter,
) {
    update(
        pending_dynamic_updates,
        voxels_per_meter,
        UpdateParams {
            center: Coords::from_world(Vec3::new(0.5, 0.5, 0.5), voxels_per_meter),
            distance_meters: 0.25,
            should_allocate_chunk: |_, _| true,
            update: move |coords, old_voxel| {
                let pos = (coords.to_world(voxels_per_meter) - Vec3::new(0.5, 0.5, 0.5)) * 8.0;
                if pos.length() >= 1.0 {
                    return old_voxel;
                }

                Voxel::new(
                    Color::linear_rgb(pos.x * 0.5 + 0.5, pos.y * 0.5 + 0.5, pos.z * 0.5 + 0.5),
                    VoxelMaterial::Solid,
                )
            },
        },
    );
}

fn perlinlike_noise_sphere(
    pending_dynamic_updates: &mut EventWriter<ChunkPendingDynamicUpdate>,
    voxels_per_meter: VoxelsPerMeter,
) {
    update(
        pending_dynamic_updates,
        voxels_per_meter,
        UpdateParams {
            center: Coords::from_world(Vec3::new(1.5, 0.5, 0.5), voxels_per_meter),
            distance_meters: 0.5,
            should_allocate_chunk: |_, _| true,
            update: move |coords, old_voxel| {
                let pos = (coords.to_world(voxels_per_meter) - Vec3::new(1.5, 0.5, 0.5)) * 4.0;
                let distance = pos.length();

                if distance >= 1.0 {
                    return old_voxel;
                }

                let noise = (pos.x * 5.0).sin() * (pos.y * 5.0).cos() * (pos.z * 5.0).sin();
                let density = (1.0 - distance) + noise * 0.2;
                if density <= 0.2 {
                    return old_voxel;
                }

                Voxel::new(
                    Color::linear_rgb((pos.x + 1.0) * 0.5, (pos.y + 1.0) * 0.5, density),
                    VoxelMaterial::Solid,
                )
            },
        },
    );
}

fn torus(
    pending_dynamic_updates: &mut EventWriter<ChunkPendingDynamicUpdate>,
    voxels_per_meter: VoxelsPerMeter,
) {
    update(
        pending_dynamic_updates,
        voxels_per_meter,
        UpdateParams {
            center: Coords::from_world(Vec3::new(2.5, 0.5, 0.5), voxels_per_meter),
            distance_meters: 0.5,
            should_allocate_chunk: |_, _| true,
            update: move |coords, old_voxel| {
                let pos = (coords.to_world(voxels_per_meter) - Vec3::new(2.5, 0.5, 0.5)) * 4.0;
                let q = Vec2::new(Vec2::new(pos.x, pos.z).length() - 0.5, pos.y).length() - 0.1;

                if q < 0.0 {
                    Voxel::new(
                        Color::linear_rgb(
                            (pos.x + 1.0) * 0.5,
                            (pos.y + 1.0) * 0.5,
                            (pos.z + 1.0) * 0.5,
                        ),
                        VoxelMaterial::Solid,
                    )
                } else {
                    old_voxel
                }
            },
        },
    );
}

fn fractal_terrain(
    pending_dynamic_updates: &mut EventWriter<ChunkPendingDynamicUpdate>,
    voxels_per_meter: VoxelsPerMeter,
) {
    update(
        pending_dynamic_updates,
        voxels_per_meter,
        UpdateParams {
            center: Coords::from_world(Vec3::new(3.5, 0.5, 0.5), voxels_per_meter),
            distance_meters: 0.75,
            should_allocate_chunk: |_, _| true,
            update: move |coords, old_voxel| {
                let pos = (coords.to_world(voxels_per_meter) - Vec3::new(3.5, 0.5, 0.5)) * 2.67;
                let mut height = 0.0;
                let mut amplitude = 0.5;
                let mut frequency = 1.0;

                for _ in 0..5 {
                    height += (pos.x * frequency).sin() * (pos.z * frequency).cos() * amplitude;
                    amplitude *= 0.5;
                    frequency *= 2.0;
                }

                if pos.y < height {
                    let color = height - pos.y;
                    Voxel::new(Color::linear_rgb(color, color, color), VoxelMaterial::Solid)
                } else {
                    old_voxel
                }
            },
        },
    );
}

fn spiral_staircase(
    pending_dynamic_updates: &mut EventWriter<ChunkPendingDynamicUpdate>,
    voxels_per_meter: VoxelsPerMeter,
) {
    update(
        pending_dynamic_updates,
        voxels_per_meter,
        UpdateParams {
            center: Coords::from_world(Vec3::new(4.5, 0.5, 0.5), voxels_per_meter),
            distance_meters: 0.5,
            should_allocate_chunk: |_, _| true,
            update: move |coords, old_voxel| {
                let pos = (coords.to_world(voxels_per_meter) - Vec3::new(4.5, 0.5, 0.5)) * 4.0;
                let angle = pos.y * std::f32::consts::PI * 2.0;
                let radius = 0.5;
                let spiral_x = angle.cos() * radius;
                let spiral_z = angle.sin() * radius;

                let dist = Vec2::new(pos.x - spiral_x, pos.z - spiral_z).length();

                if dist < 0.2 && (pos.y * 8.0) % 1.0 < 0.25 {
                    Voxel::new(
                        Color::linear_rgb(
                            (pos.x + 1.0) * 0.5,
                            (pos.y + 1.0) * 0.5,
                            (pos.z + 1.0) * 0.5,
                        ),
                        VoxelMaterial::Solid,
                    )
                } else {
                    old_voxel
                }
            },
        },
    );
}
