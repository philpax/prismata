use bevy::{
    picking::{
        events::Click,
        mesh_picking::MeshPickingPlugin,
        prelude::*,
    },
    prelude::*,
};
use transform_gizmo_bevy::GizmoTarget;

use crate::{ui::CursorVisible, util};

#[derive(Component)]
/// When attached to an entity, all [`Mesh`] children of the entity will be pickable.
/// Used to identify the root entity when a child mesh is clicked.
pub struct PickableChildren;

#[derive(Component)]
/// Tracks whether an entity is currently selected
pub struct Selected;

pub fn plugin(app: &mut App) {
    app.add_plugins(MeshPickingPlugin)
        .add_systems(Update, (handle_selection_clicks, update_picking).chain());
}

/// Handle click events to toggle selection
fn handle_selection_clicks(
    mut click_events: MessageReader<Pointer<Click>>,
    mut commands: Commands,
    cursor_visible: Res<CursorVisible>,
    selected_query: Query<(), With<Selected>>,
    parent_query: Query<&ChildOf>,
    pickable_children_query: Query<(), With<PickableChildren>>,
) {
    // Don't process selection clicks when cursor is hidden (e.g., during camera control)
    if !cursor_visible.0 {
        return;
    }

    for click in click_events.read() {
        let mut entity = click.entity;

        // If clicked entity is a child of a PickableChildren entity, select the parent instead
        if let Some(parent) =
            util::find_parent_with_component(&parent_query, &pickable_children_query, entity)
        {
            entity = parent;
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
    target_query: Query<Entity, With<GizmoTarget>>,
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
