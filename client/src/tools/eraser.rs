use web_time::Duration;

use bevy::{camera::visibility::RenderLayers, picking::prelude::Pickable, prelude::*};
use bevy_egui::egui;

use crate::{
    raycast::{update_world_rayhits, CursorRayHit, RaycastIgnore},
    rendering::{AlphaPulse, MAIN_CAMERA_ONLY_LAYER},
    ui::CursorVisible,
    voxel,
};

use super::{
    brush_should_be_created, brush_should_be_destroyed, brush_size_range, is_in_use,
    scroll_size_system, started_being_used, stopped_being_used, Tool,
};

#[derive(Resource)]
pub struct OurEraserBrush(pub Entity);

#[derive(Resource)]
pub struct OurEraserBrushSettings {
    pub radius: f32,
}

pub fn plugin(app: &mut App) {
    app.insert_resource(OurEraserBrushSettings { radius: 0.0 })
        .add_systems(
            Update,
            (
                scroll_size_system(Tool::Eraser, |s: &mut OurEraserBrushSettings| &mut s.radius),
                create_brush.run_if(brush_should_be_created::<OurEraserBrush>(Tool::Eraser)),
                destroy_brush.run_if(brush_should_be_destroyed::<OurEraserBrush>(Tool::Eraser)),
                update_brush_viz.run_if(resource_exists::<OurEraserBrush>),
                on_use_start.run_if(started_being_used(Tool::Eraser)),
                on_use.run_if(is_in_use(Tool::Eraser, Some(Duration::from_millis(30)))),
                on_use_end.run_if(stopped_being_used(Tool::Eraser)),
            )
                .chain()
                .after(update_world_rayhits),
        );
}

fn create_brush(
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let id = commands
        .spawn((
            Name::new("Eraser Brush"),
            Mesh3d(meshes.add(Sphere { radius: 1.0 }.mesh().ico(5).unwrap())),
            MeshMaterial3d(materials.add(StandardMaterial {
                alpha_mode: AlphaMode::Blend,
                base_color: Color::linear_rgba(1.0, 0.0, 0.0, 1.0),
                unlit: true,
                ..default()
            })),
            Transform::default(),
            Visibility::Hidden,
            AlphaPulse::new(0.25, 0.5),
            RenderLayers::layer(MAIN_CAMERA_ONLY_LAYER as usize),
            RaycastIgnore,
            Pickable::IGNORE,
        ))
        .id();
    commands.insert_resource(OurEraserBrush(id));
}

fn destroy_brush(brush: Res<OurEraserBrush>, mut commands: Commands) {
    commands.entity(brush.0).despawn();
    commands.remove_resource::<OurEraserBrush>();
}

fn update_brush_viz(
    voxel_under_cursor: Res<CursorRayHit>,
    cursor_visible: Res<CursorVisible>,
    our_eraser_brush: Res<OurEraserBrush>,
    voxels_per_meter: Res<voxel::VoxelsPerMeter>,
    settings: Res<OurEraserBrushSettings>,
    mut tool_brushes: Query<(&mut Transform, &mut Visibility)>,
) {
    let (mut transform, mut visibility) = tool_brushes.get_mut(our_eraser_brush.0).unwrap();
    if let Some(voxel) = voxel_under_cursor.ray_hit.filter(|_| cursor_visible.0) {
        transform.translation = voxel.entry_coords.to_world(*voxels_per_meter);
        transform.scale = Vec3::splat(settings.radius);
        *visibility = Visibility::Visible;
    } else {
        *visibility = Visibility::Hidden;
    }
}

fn on_use_start() {}

fn on_use(
    cursor_ray_hit: Res<CursorRayHit>,
    settings: Res<OurEraserBrushSettings>,
    voxels_per_meter: Res<voxel::VoxelsPerMeter>,
    mut pending_dynamic_updates: MessageWriter<voxel::ChunkPendingDynamicUpdate>,
) {
    let Some(center) = cursor_ray_hit.coords() else {
        return;
    };

    let radius = settings.radius;
    let radius_voxel = (radius * voxels_per_meter.0 as f32) as u32;
    let radius_voxel_sqr = radius_voxel * radius_voxel;

    // TODO: This will fail because we can issue multiple updates within the same frame.
    // Instead, we should queue up the updates and apply them all at once.
    voxel::update(
        &mut pending_dynamic_updates,
        *voxels_per_meter,
        voxel::UpdateParams {
            center,
            distance_meters: radius,
            should_allocate_chunk: |_, _| false,
            update: move |coords, old_voxel| {
                let distance_sqr = center.distance_squared(coords);
                if distance_sqr <= radius_voxel_sqr {
                    voxel::Voxel::default()
                } else {
                    old_voxel
                }
            },
        },
    );
}

fn on_use_end() {}

pub fn ui_top_left_panel(ui: &mut egui::Ui, world: &mut World) {
    let range = brush_size_range(world);
    let mut settings = world.resource_mut::<OurEraserBrushSettings>();
    ui.add(
        egui::Slider::new(&mut settings.radius, range)
            .fixed_decimals(2)
            .text("Radius"),
    );
}
