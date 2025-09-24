use bevy::prelude::*;
use bevy_mod_picking::{
    picking_core::PickingPluginsSettings, prelude::*, selection::SelectionPluginSettings,
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

pub fn plugin(app: &mut App) {
    app.add_plugins(DefaultPickingPlugins)
        .insert_resource(SelectionPluginSettings {
            click_nothing_deselect_all: false,
            ..default()
        })
        .add_systems(
            PreUpdate,
            (process_pickable_children, toggle_picking_enabled).chain(),
        )
        .add_systems(Update, update_picking);
}

fn process_pickable_children(
    pickable_children_query: Query<Entity, With<PickableChildren>>,
    children_query: Query<&Children>,
    mesh_query: Query<&Handle<Mesh>, Without<Pickable>>,
    mut commands: Commands,
) {
    fn add_pickable(
        entity: Entity,
        children_query: &Query<&Children>,
        mesh_query: &Query<&Handle<Mesh>, Without<Pickable>>,
        commands: &mut Commands,
    ) {
        if mesh_query.contains(entity) {
            commands
                .entity(entity)
                .insert((PickableBundle::default(), PickableChild));
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
    mut picking_settings: ResMut<PickingPluginsSettings>,
) {
    // Picking is disabled when any of the gizmos is focused or active.
    picking_settings.is_enabled = gizmo_targets
        .iter()
        .all(|target| !target.is_focused() && !target.is_active())
        && cursor_visible.0
        && active_tool.is_none();
}

/// Continuously update entities based on their picking state
fn update_picking(
    mut commands: Commands,
    pick_query: Query<(Entity, &PickSelection, Has<PickableChild>)>,
    target_query: Query<(), With<GizmoTarget>>,

    parent_query: Query<&Parent>,
    children_query: Query<&Children>,
    pickable_children_query: Query<(), With<PickableChildren>>,
) {
    fn get_pick_selected(
        pick_query: &Query<(Entity, &PickSelection, Has<PickableChild>)>,
        children_query: &Query<&Children>,
        entity: Entity,
    ) -> bool {
        if pick_query
            .get(entity)
            .is_ok_and(|(_, ps, _)| ps.is_selected)
        {
            return true;
        }

        children_query.get(entity).is_ok_and(|c| {
            c.iter()
                .any(|child| get_pick_selected(pick_query, children_query, *child))
        })
    }

    for (entity, pick_interaction, is_pickable_child) in &pick_query {
        let (entity, is_selected) = if is_pickable_child {
            let parent = match util::find_parent_with_component(
                &parent_query,
                &pickable_children_query,
                entity,
            ) {
                Some(parent) => parent,
                None => continue,
            };

            (
                parent,
                get_pick_selected(&pick_query, &children_query, parent),
            )
        } else {
            (entity, pick_interaction.is_selected)
        };

        let mut entity_cmd = commands.entity(entity);
        let has_gizmo_target = target_query.contains(entity);
        if is_selected && !has_gizmo_target {
            entity_cmd.insert(GizmoTarget::default());
        } else if !is_selected && has_gizmo_target {
            entity_cmd.remove::<GizmoTarget>();
        }
    }
}
