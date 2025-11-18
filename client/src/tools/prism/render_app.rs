use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use bevy::{
    core_pipeline::core_3d::graph::{Core3d, Node3d},
    ecs::query::QueryItem,
    prelude::*,
    render::{
        render_graph::{
            NodeRunError, RenderGraphContext, RenderGraphExt, RenderLabel, ViewNode, ViewNodeRunner,
        },
        render_resource::{
            Buffer, BufferDescriptor, BufferUsages, Extent3d, MapMode,
        },
        renderer::{RenderContext, RenderDevice},
        view::{ViewDepthTexture, ViewTarget},
        Render, RenderSystems,
    },
};
use wgpu::{TexelCopyBufferInfo, TexelCopyBufferLayout};

use super::{
    PrismCapturePayload, PrismMainCamera, PrismMaskCamera, PrismPostProcessRender, PrismRenderSize,
};

pub fn plugin(render_app: &mut App) {
    render_app
        .add_systems(
            Render,
            create_prism_buffers.run_if(resource_exists_and_changed::<PrismRenderSize>),
        )
        .add_render_graph_node::<ViewNodeRunner<PrismMainPostProcessNode>>(
            Core3d,
            PrismMainPostProcessLabel,
        )
        .add_render_graph_node::<ViewNodeRunner<PrismMaskPostProcessNode>>(
            Core3d,
            PrismMaskPostProcessLabel,
        )
        .add_render_graph_edges(
            Core3d,
            (
                Node3d::Tonemapping,
                PrismMainPostProcessLabel,
                PrismMaskPostProcessLabel,
                Node3d::EndMainPassPostProcessing,
            ),
        )
        .add_systems(
            Render,
            map_buffers.after(RenderSystems::Render).run_if(
                resource_exists::<PrismBuffers>.and(resource_exists::<PrismPostProcessRender>),
            ),
        );
}

#[derive(Resource, Clone)]
struct PrismBuffers {
    render: Buffer,
    depth: Buffer,
    mask: Buffer,
    mapped: Arc<AtomicBool>,
}
fn create_prism_buffers(
    render_size: Res<PrismRenderSize>,
    render_device: Res<RenderDevice>,
    mut commands: Commands,
) {
    let render = render_device.create_buffer(&BufferDescriptor {
        label: Some("Prism Render (CPU copy)"),
        size: (render_size.x * render_size.y) as u64 * std::mem::size_of::<[u8; 4]>() as u64,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let depth = render_device.create_buffer(&BufferDescriptor {
        label: Some("Prism Depth (CPU copy)"),
        size: (render_size.x * render_size.y) as u64 * std::mem::size_of::<f32>() as u64,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mask = render_device.create_buffer(&BufferDescriptor {
        label: Some("Prism Mask (CPU copy)"),
        size: (render_size.x * render_size.y) as u64 * std::mem::size_of::<[u8; 4]>() as u64,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    commands.insert_resource(PrismBuffers {
        render,
        depth,
        mask,
        mapped: Arc::new(AtomicBool::new(false)),
    });
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct PrismMainPostProcessLabel;

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct PrismMaskPostProcessLabel;

#[derive(Default)]
struct PrismMainPostProcessNode;
impl ViewNode for PrismMainPostProcessNode {
    type ViewQuery = (
        &'static ViewTarget,
        // This is not used by the node itself, but is necessary to ensure that
        // we do not copy data from the main camera
        &'static PrismMainCamera,
        &'static ViewDepthTexture,
    );

    fn run(
        &self,
        _graph: &mut RenderGraphContext,
        render_context: &mut RenderContext,
        (view_target, _prism_camera, view_depth_texture): QueryItem<Self::ViewQuery>,
        world: &World,
    ) -> Result<(), NodeRunError> {
        let Some(render_size) = world.get_resource::<PrismRenderSize>() else {
            return Ok(());
        };
        let Some(buffers) = world.get_resource::<PrismBuffers>() else {
            return Ok(());
        };

        if buffers.mapped.load(Ordering::Relaxed) {
            info!("Buffers are mapped and can't be copied to, skipping");
            return Ok(());
        }

        let command_encoder = render_context.command_encoder();
        command_encoder.copy_texture_to_buffer(
            view_target.main_texture().as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &buffers.render,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(render_size.x * std::mem::size_of::<[u8; 4]>() as u32),
                    rows_per_image: None,
                },
            },
            Extent3d {
                width: render_size.x,
                height: render_size.y,
                depth_or_array_layers: 1,
            },
        );
        command_encoder.copy_texture_to_buffer(
            view_depth_texture.texture.as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &buffers.depth,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(render_size.x * std::mem::size_of::<f32>() as u32),
                    rows_per_image: None,
                },
            },
            Extent3d {
                width: render_size.x,
                height: render_size.y,
                depth_or_array_layers: 1,
            },
        );

        Ok(())
    }
}

#[derive(Default)]
struct PrismMaskPostProcessNode;
impl ViewNode for PrismMaskPostProcessNode {
    type ViewQuery = (
        &'static ViewTarget,
        // This is not used by the node itself, but is necessary to ensure that
        // we do not copy data from the main camera
        &'static PrismMaskCamera,
    );

    fn run(
        &self,
        _graph: &mut RenderGraphContext,
        render_context: &mut RenderContext,
        (view_target, _prism_camera): QueryItem<Self::ViewQuery>,
        world: &World,
    ) -> Result<(), NodeRunError> {
        let Some(render_size) = world.get_resource::<PrismRenderSize>() else {
            return Ok(());
        };
        let Some(buffers) = world.get_resource::<PrismBuffers>() else {
            return Ok(());
        };

        if buffers.mapped.load(Ordering::Relaxed) {
            info!("Buffers are mapped and can't be copied to, skipping");
            return Ok(());
        }

        let command_encoder = render_context.command_encoder();
        command_encoder.copy_texture_to_buffer(
            view_target.main_texture().as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &buffers.mask,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(render_size.x * std::mem::size_of::<[u8; 4]>() as u32),
                    rows_per_image: None,
                },
            },
            Extent3d {
                width: render_size.x,
                height: render_size.y,
                depth_or_array_layers: 1,
            },
        );

        Ok(())
    }
}

fn map_buffers(
    render_device: Res<RenderDevice>,
    buffers: Res<PrismBuffers>,
    post_process_render: Res<PrismPostProcessRender>,
) {
    let Ok((camera_position, camera_rotation, camera_projection, near_plane, far_plane)) =
        post_process_render.capture_rx.try_recv()
    else {
        // No capture requests in the pipeline
        return;
    };

    info!("Received sender");

    if buffers.mapped.load(Ordering::Relaxed) {
        info!("Buffers are already mapped, ignoring duplicate request");
        return;
    }

    let buffers = buffers.clone();
    let render_device = render_device.clone();
    let post_process_render = post_process_render.clone();
    let future = async move {
        buffers.mapped.store(true, Ordering::Relaxed);

        let (render_signal_tx, render_signal_rx) =
            futures_intrusive::channel::shared::oneshot_channel();
        let render_buffer_slice = buffers.render.slice(..);
        render_buffer_slice.map_async(MapMode::Read, move |r| match r {
            Ok(_) => render_signal_tx
                .send(())
                .expect("Failed to send render map update"),
            Err(err) => panic!("Failed to map render buffer {err}"),
        });

        let (depth_signal_tx, depth_signal_rx) =
            futures_intrusive::channel::shared::oneshot_channel();
        let depth_buffer_slice = buffers.depth.slice(..);
        depth_buffer_slice.map_async(MapMode::Read, move |r| match r {
            Ok(_) => depth_signal_tx
                .send(())
                .expect("Failed to send render map update"),
            Err(err) => panic!("Failed to map render buffer {err}"),
        });

        let (mask_signal_tx, mask_signal_rx) =
            futures_intrusive::channel::shared::oneshot_channel();
        let mask_buffer_slice = buffers.mask.slice(..);
        mask_buffer_slice.map_async(MapMode::Read, move |r| match r {
            Ok(_) => mask_signal_tx
                .send(())
                .expect("Failed to send mask map update"),
            Err(err) => panic!("Failed to map mask buffer {err}"),
        });

        render_device.poll(wgpu::PollType::Wait).unwrap();
        info!("Polled device");

        let mut payload = PrismCapturePayload {
            camera_position,
            camera_rotation,
            camera_projection,
            near_plane,
            far_plane,
            ..default()
        };

        render_signal_rx
            .receive()
            .await
            .expect("Failed to receive render map update");
        payload.render = render_buffer_slice.get_mapped_range().to_vec();
        buffers.render.unmap();

        depth_signal_rx
            .receive()
            .await
            .expect("Failed to receive render map update");
        payload.depth = depth_buffer_slice.get_mapped_range().to_vec();
        buffers.depth.unmap();

        mask_signal_rx
            .receive()
            .await
            .expect("Failed to receive mask map update");
        payload.mask = mask_buffer_slice.get_mapped_range().to_vec();
        buffers.mask.unmap();

        post_process_render
            .result_tx
            .send(payload)
            .expect("Failed to send data to main world");
        buffers.mapped.store(false, Ordering::Relaxed);
    };

    #[cfg(not(target_arch = "wasm32"))]
    bevy::tasks::block_on(future);
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_futures::spawn_local(future);
}
