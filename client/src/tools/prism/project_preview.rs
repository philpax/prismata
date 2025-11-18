use bevy::{
    hierarchy::Parent,
    prelude::*,
    render::{mesh::Indices, render_asset::RenderAssetUsages, render_resource::*},
};

use crate::{
    raycast::RaycastIgnore,
    voxel::{self, VoxelSizeMeters},
};

use super::{project::ProjectionRequest, PrismState};

pub fn plugin(app: &mut App) {
    app.insert_resource(StateChangeHysteresisInstant(None))
        .insert_resource(PrismPreviewVizEntity(None))
        .add_systems(
            Update,
            (trigger_state_change, regenerate_entity_on_state_change)
                .chain()
                .run_if(resource_exists::<PrismState>),
        );
}

#[derive(Resource)]
struct PrismPreviewVizEntity(Option<Entity>);

#[derive(Resource)]
/// Bevy's renderer crashes when we try to update the data on successive frames. I'm not really
/// sure why this is - surely it should be copying the data out properly - but in any case,
/// we need to get this working, so this acts as a hysteresis period in which the request to
/// update the state is delayed so that only one change will be applied.
struct StateChangeHysteresisInstant(Option<(web_time::Instant, &'static str)>);

fn trigger_state_change(
    state: Res<PrismState>,
    mut hysteresis: ResMut<StateChangeHysteresisInstant>,
) {
    if state.is_changed()
        && (hysteresis.0.is_none() || hysteresis.0.is_some_and(|t| t.1 != state.id()))
    {
        hysteresis.0 = Some((web_time::Instant::now(), state.id()));
    }
}

fn regenerate_entity_on_state_change(
    state: Res<PrismState>,
    voxel_size_meters: Res<VoxelSizeMeters>,
    global_transform_query: Query<&GlobalTransform>,
    parent_query: Query<&Parent>,
    mut preview_viz_entity: ResMut<PrismPreviewVizEntity>,
    mut hysteresis: ResMut<StateChangeHysteresisInstant>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if hysteresis.0.is_none()
        || hysteresis
            .0
            .is_some_and(|t| t.0.elapsed() < web_time::Duration::from_millis(100))
    {
        return;
    }

    if let Some(entity) = preview_viz_entity.0.take() {
        if let Ok(parent) = parent_query.get(entity) {
            commands.entity(parent.get()).remove_children(&[entity]);
        }
        if let Some(entity) = commands.get_entity(entity) {
            // Should be taken care of by the preview entity being despawned,
            // but just in case
            entity.despawn_descendants_recursive();
        }
    }
    if let PrismState::Rendered(rendered) = &*state {
        info!("Updating preview entity");
        let global_transform = global_transform_query.get(rendered.preview_entity).unwrap();
        let global_transform_inv = Mat4::from(global_transform.affine()).inverse();

        let mut splats = ProjectionRequest::from_rendered(rendered, global_transform)
            .calculate_data(*voxel_size_meters);
        // TODO: change this so that this is no longer necessary - `ProjectionRequest` should
        // already be in cameraspace and do the multiplication into worldspace at projection time
        //
        // ...although all of this should probably also live on the GPU. Ship first, optimise later.
        for splat in &mut splats {
            splat.position = global_transform_inv.transform_point3(splat.position);
        }

        let id = commands
            .spawn((
                Mesh3d(meshes.add(generate_splat_mesh(&splats))),
                MeshMaterial3d(materials.add(StandardMaterial {
                    unlit: true,
                    ..default()
                })),
                Transform::default(),
                RaycastIgnore,
            ))
            .id();
        commands
            .entity(rendered.preview_entity)
            .add_children(&[id]);
        preview_viz_entity.0 = Some(id);
    };

    hysteresis.0 = None;
}

fn generate_splat_mesh(splats: &[voxel::Splat]) -> Mesh {
    let mut positions = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::new();

    for (i, splat) in splats.iter().enumerate() {
        let base_cube = create_cube_vertices(splat.position, splat.size);
        positions.extend_from_slice(&base_cube);

        let color_array = splat.color.extend(1.0).to_array();
        colors.extend(std::iter::repeat_n(color_array, 24));

        let base_index = (i * 24) as u32;
        let cube_indices = [
            0, 3, 1, 1, 3, 2, 4, 5, 7, 5, 6, 7, 8, 11, 9, 9, 11, 10, 12, 13, 15, 13, 14, 15, 16,
            19, 17, 17, 19, 18, 20, 21, 23, 21, 22, 23,
        ];
        indices.extend(cube_indices.iter().map(|&idx| base_index + idx));
    }

    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(Indices::U32(indices))
}

fn create_cube_vertices(position: Vec3, size: f32) -> [[f32; 3]; 24] {
    let half_size = size / 2.0;
    [
        // top (facing towards +y)
        [
            position.x - half_size,
            position.y + half_size,
            position.z - half_size,
        ],
        [
            position.x + half_size,
            position.y + half_size,
            position.z - half_size,
        ],
        [
            position.x + half_size,
            position.y + half_size,
            position.z + half_size,
        ],
        [
            position.x - half_size,
            position.y + half_size,
            position.z + half_size,
        ],
        // bottom (-y)
        [
            position.x - half_size,
            position.y - half_size,
            position.z - half_size,
        ],
        [
            position.x + half_size,
            position.y - half_size,
            position.z - half_size,
        ],
        [
            position.x + half_size,
            position.y - half_size,
            position.z + half_size,
        ],
        [
            position.x - half_size,
            position.y - half_size,
            position.z + half_size,
        ],
        // right (+x)
        [
            position.x + half_size,
            position.y - half_size,
            position.z - half_size,
        ],
        [
            position.x + half_size,
            position.y - half_size,
            position.z + half_size,
        ],
        [
            position.x + half_size,
            position.y + half_size,
            position.z + half_size,
        ],
        [
            position.x + half_size,
            position.y + half_size,
            position.z - half_size,
        ],
        // left (-x)
        [
            position.x - half_size,
            position.y - half_size,
            position.z - half_size,
        ],
        [
            position.x - half_size,
            position.y - half_size,
            position.z + half_size,
        ],
        [
            position.x - half_size,
            position.y + half_size,
            position.z + half_size,
        ],
        [
            position.x - half_size,
            position.y + half_size,
            position.z - half_size,
        ],
        // back (+z)
        [
            position.x - half_size,
            position.y - half_size,
            position.z + half_size,
        ],
        [
            position.x - half_size,
            position.y + half_size,
            position.z + half_size,
        ],
        [
            position.x + half_size,
            position.y + half_size,
            position.z + half_size,
        ],
        [
            position.x + half_size,
            position.y - half_size,
            position.z + half_size,
        ],
        // forward (-z)
        [
            position.x - half_size,
            position.y - half_size,
            position.z - half_size,
        ],
        [
            position.x - half_size,
            position.y + half_size,
            position.z - half_size,
        ],
        [
            position.x + half_size,
            position.y + half_size,
            position.z - half_size,
        ],
        [
            position.x + half_size,
            position.y - half_size,
            position.z - half_size,
        ],
    ]
}
