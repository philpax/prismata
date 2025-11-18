use bevy::{hierarchy::Parent, prelude::*};

/// Given an entity and the relevant queries, find the first parent of the entity that has
/// the specified component. Includes the entity itself.
pub fn find_parent_with_component<T: Component>(
    parent_query: &Query<&Parent>,
    component_query: &Query<(), With<T>>,
    entity: Entity,
) -> Option<Entity> {
    if component_query.contains(entity) {
        Some(entity)
    } else {
        parent_query.get(entity).ok().and_then(|parent| {
            find_parent_with_component(parent_query, component_query, parent.get())
        })
    }
}
