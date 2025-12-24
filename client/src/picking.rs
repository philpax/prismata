use bevy::{
    picking::{events::Click, mesh_picking::MeshPickingPlugin, prelude::*},
    prelude::*,
};
use transform_gizmo_bevy::GizmoTarget;

use crate::{ui::CursorVisible, util};

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
    // Note: DefaultPickingPlugins is now included in DefaultPlugins as of Bevy 0.15+
    app.add_plugins(MeshPickingPlugin)
        .add_systems(PreUpdate, process_pickable_children)
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
                add_pickable(child, children_query, mesh_query, commands);
            }
        }
    }

    for entity in pickable_children_query.iter() {
        add_pickable(entity, &children_query, &mesh_query, &mut commands);
    }
}

/// Handle click events to toggle selection
fn handle_selection_clicks(
    mut click_events: MessageReader<Pointer<Click>>,
    mut commands: Commands,
    cursor_visible: Res<CursorVisible>,
    selected_query: Query<(), With<Selected>>,
    pickable_child_query: Query<(), With<PickableChild>>,
    parent_query: Query<&ChildOf>,
    pickable_children_query: Query<(), With<PickableChildren>>,
) {
    // Don't process selection clicks when cursor is hidden (e.g., during camera control)
    if !cursor_visible.0 {
        return;
    }

    for click in click_events.read() {
        let mut entity = click.entity;

        // If this is a pickable child, find its parent
        if pickable_child_query.contains(entity) {
            if let Some(parent) =
                util::find_parent_with_component(&parent_query, &pickable_children_query, entity)
            {
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
