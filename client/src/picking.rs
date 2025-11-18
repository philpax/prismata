use bevy::{
    picking::{events::Click, mesh_picking::MeshPickingPlugin, pointer::PointerId, prelude::*},
    prelude::*,
};
use transform_gizmo_bevy::GizmoTarget;

use crate::{tools::ActiveTool, ui::CursorVisible, util};

#[derive(Component)]
/// When attached to an entity, all [`Mesh`] children of the entity will be pickable.
pub struct PickableChildren;

#[derive(Component)]
/// Attached by [`process_pickable_children`] to all entities made pickable as a result
/// of pickable parents. Used to determine that the root should be used as the gizmo target.
struct PickableChild;

#[derive(Component)]
/// Tracks whether an entity is currently selected
pub struct Selected;

pub fn plugin(app: &mut App) {
    app.add_plugins((DefaultPickingPlugins, MeshPickingPlugin))
        .add_systems(
            PreUpdate,
            (process_pickable_children, toggle_picking_enabled).chain(),
        )
        .add_systems(Update, (handle_selection_clicks, update_picking).chain());
}

fn process_pickable_children(
    pickable_children_query: Query<Entity, With<PickableChildren>>,
    children_query: Query<&Children>,
    mesh_query: Query<&Mesh3d, Without<Pickable>>,
    mut commands: Commands,
) {
    fn add_pickable(
        entity: Entity,
        children_query: &Query<&Children>,
        mesh_query: &Query<&Mesh3d, Without<Pickable>>,
        commands: &mut Commands,
    ) {
        if mesh_query.contains(entity) {
            commands
                .entity(entity)
                .insert((Pickable::default(), PickableChild));
        }

        if let Ok(children) = children_query.get(entity) {
            for child in children.iter() {
                add_pickable(*child, children_query, mesh_query, commands);
            }
        }
    }

    for entity in pickable_children_query.iter() {
        add_pickable(entity, &children_query, &mesh_query, &mut commands);
    }
}

fn toggle_picking_enabled(
    gizmo_targets: Query<&GizmoTarget>,
    cursor_visible: Res<CursorVisible>,
    active_tool: Res<ActiveTool>,
    mut picking_settings: ResMut<PickingSettings>,
) {
    // Picking is disabled when any of the gizmos is focused or active.
    picking_settings.is_enabled = gizmo_targets
        .iter()
        .all(|target| !target.is_focused() && !target.is_active())
        && cursor_visible.0
        && active_tool.is_none();
}

/// Handle click events to toggle selection
fn handle_selection_clicks(
    mut click_events: EventReader<Pointer<Click>>,
    mut commands: Commands,
    selected_query: Query<(), With<Selected>>,
    pickable_child_query: Query<(), With<PickableChild>>,
    parent_query: Query<&ChildOf>,
    pickable_children_query: Query<(), With<PickableChildren>>,
) {
    for click in click_events.read() {
        let mut entity = click.target;

        // If this is a pickable child, find its parent
        if pickable_child_query.contains(entity) {
            if let Some(parent) = util::find_parent_with_component(
                &parent_query,
                &pickable_children_query,
                entity,
            ) {
                entity = parent;
            }
        }

        // Toggle selection
        if selected_query.contains(entity) {
            commands.entity(entity).remove::<Selected>();
        } else {
            commands.entity(entity).insert(Selected);
        }
    }
}

/// Continuously update entities based on their picking state
fn update_picking(
    mut commands: Commands,
    selected_query: Query<Entity, With<Selected>>,
    target_query: Query<(), With<GizmoTarget>>,
) {
    for entity in &selected_query {
        let has_gizmo_target = target_query.contains(entity);
        if !has_gizmo_target {
            commands.entity(entity).insert(GizmoTarget::default());
        }
    }

    // Remove gizmo targets from entities that are no longer selected
    for entity in &target_query {
        if !selected_query.contains(entity) {
            commands.entity(entity).remove::<GizmoTarget>();
        }
    }
}
