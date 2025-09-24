#![allow(dead_code)]

use prismata_protocol::tools::prism::{RenderInput, RenderOutput};
use rucomfyui::{
    nodes::{
        all::*,
        loaders::out::CheckpointLoaderSimpleOutput,
        types::{ConditioningOut, FloatOut, ImageOut, LatentOut, MaskOut, ModelOut, UntypedOut},
    },
    workflow::{WorkflowInput, WorkflowMeta, WorkflowNode, WorkflowNodeId},
    WorkflowGraph,
};

use crate::comfyui::{Error, WorkflowSettings};

const DEPTH_FILENAME: &str = "depth_input.png";
const RENDER_FILENAME: &str = "render_input.png";
const MASK_FILENAME: &str = "mask_input.png";
const DEPTHPRO_FILENAME: &str = "depthpro_input.png";
const API_WORKFLOW_FILENAME: &str = "api_workflow.json";

/// Used to ensure that only one request can upload images at a time, avoiding issues with
/// future requests overwriting the images of the current request.
static QUEUE_SEMAPHORE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

pub async fn queue_prompt(
    client: &rucomfyui::Client,
    workflow_settings: &WorkflowSettings,
    input: &RenderInput,
) -> Result<RenderOutput, Error> {
    let _permit = QUEUE_SEMAPHORE.acquire().await.unwrap();

    upload_image(
        client,
        &data_encoding::BASE64.decode(input.depth.as_bytes())?,
        DEPTH_FILENAME,
    )
    .await?;
    upload_image(
        client,
        &data_encoding::BASE64.decode(input.base_render.as_bytes())?,
        RENDER_FILENAME,
    )
    .await?;
    upload_image(
        client,
        &data_encoding::BASE64.decode(input.mask.as_bytes())?,
        MASK_FILENAME,
    )
    .await?;

    let (graph, preview_image, depthpro) = if input.repaint_amount <= 0.0 {
        depth_only_workflow(workflow_settings.enable_depthpro)
    } else {
        // Queue prompt
        workflow(
            &input.prompt,
            input.seed,
            input.repaint_amount,
            input.sampler_steps,
            input.depth_controlnet_strength,
            workflow_settings.enable_depthpro,
        )
    };

    if workflow_settings.dump_workflow {
        let mut file = std::fs::File::create(API_WORKFLOW_FILENAME).unwrap();
        serde_json::to_writer_pretty(&mut file, &graph.clone().into_workflow()).unwrap();
    }

    let output = client.easy_queue(&graph.into_workflow()).await?;

    let diffuse = data_encoding::BASE64.encode(&encode_image_to_memory(&image::load_from_memory(
        &output[&preview_image].images[0],
    )?)?);

    let depth = if let Some(depthpro_id) = depthpro {
        let focal_length = output[&depthpro_id].texts[0].parse::<f32>()?;
        // We pass the image through as-is as we don't want to potentially damage the data
        // in the re-encode
        Some((
            data_encoding::BASE64.encode(&output[&depthpro_id].images[0]),
            focal_length,
        ))
    } else {
        None
    };

    Ok(RenderOutput {
        render_id: input.render_id,
        diffuse,
        depth,
    })
}

async fn upload_image(
    client: &rucomfyui::Client,
    image_data: &[u8],
    filename: &str,
) -> Result<(), Error> {
    let _decoder = image::codecs::png::PngDecoder::new(std::io::Cursor::new(image_data))?;
    client.upload(filename, image_data.to_owned()).await?;
    Ok(())
}

fn encode_image_to_memory(img: &image::DynamicImage) -> image::ImageResult<Vec<u8>> {
    let mut image_data = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut image_data),
        image::ImageFormat::Png,
    )?;
    Ok(image_data)
}

fn workflow(
    prompt: &str,
    seed: i64,
    repaint_amount: f32,
    sampler_steps: u8,
    depth_controlnet_strength: f32,
    with_depthpro: bool,
) -> (WorkflowGraph, WorkflowNodeId, Option<WorkflowNodeId>) {
    let g = WorkflowGraph::default();

    let CheckpointLoaderSimpleOutput { model, vae, clip } = g.add(CheckpointLoaderSimple::new(
        "sd_xl_base_1.0_0.9vae.safetensors",
    ));

    let load_inpaint_id = g.add_dynamic(
        WorkflowNode::new("INPAINT_LoadFooocusInpaint")
            .with_input("head", "fooocus_inpaint_head.pth")
            .with_input("patch", "inpaint_v26.fooocus.patch")
            .with_meta(WorkflowMeta::new("Load Fooocus Inpaint")),
    );
    let expand_mask_id = g.add_typed_dynamic::<MaskOut>(
        WorkflowNode::new("INPAINT_ExpandMask")
            .with_input("grow", 16)
            .with_input("blur", 7)
            .with_input("mask", g.add(LoadImageMask::new(MASK_FILENAME, "red")))
            .with_meta(WorkflowMeta::new("Expand Mask")),
    );
    let vae_encode_and_inpaint_conditioning_id =
        g.add_typed_dynamic::<(ConditioningOut, ConditioningOut, LatentOut, LatentOut)>(
            WorkflowNode::new("INPAINT_VAEEncodeInpaintConditioning")
                .with_input(
                    "positive",
                    g.add(ControlNetApply {
                        strength: depth_controlnet_strength,
                        conditioning: g.add(ClipTextEncode::new(prompt, clip)),
                        control_net: g.add(SetUnionControlNetType {
                            control_net: g.add(ControlNetLoader::new(
                                "diffusion_pytorch_model_promax.safetensors",
                            )),
                            type_: "depth",
                        }),
                        image: g.add(LoadImage::new(DEPTH_FILENAME)).image,
                    }),
                )
                .with_input("negative", g.add(ClipTextEncode::new("", clip)))
                .with_input("vae", vae)
                .with_input(
                    "pixels",
                    g.add(ImageScale {
                        upscale_method: "nearest-exact",
                        width: 1024,
                        height: 1024,
                        crop: "disabled",
                        image: g.add(LoadImage::new(RENDER_FILENAME)).image,
                    }),
                )
                .with_input("mask", expand_mask_id)
                .with_meta(WorkflowMeta::new("VAE Encode & Inpaint Conditioning")),
        );

    let model = g.add_typed_dynamic::<ModelOut>(
        WorkflowNode::new("INPAINT_ApplyFooocusInpaint")
            .with_input("model", model)
            .with_input("patch", WorkflowInput::slot(load_inpaint_id, 0))
            .with_input("latent", vae_encode_and_inpaint_conditioning_id.2)
            .with_meta(WorkflowMeta::new("Apply Fooocus Inpaint")),
    );

    let vae_decode = g.add(VaeDecode {
        samples: g.add(KSampler {
            seed,
            cfg: 5.0,
            steps: sampler_steps as u32,
            sampler_name: "dpmpp_2m_sde_gpu",
            scheduler: "karras",
            denoise: repaint_amount,

            model,
            positive: vae_encode_and_inpaint_conditioning_id.0,
            negative: vae_encode_and_inpaint_conditioning_id.1,
            latent_image: vae_encode_and_inpaint_conditioning_id.3,
        }),
        vae,
    });

    let preview_image = g.add(PreviewImage::new(vae_decode));

    let preview_image_depth = with_depthpro.then(|| depthpro_nodes(&g, vae_decode.into()));

    (g, preview_image, preview_image_depth)
}

fn depth_only_workflow(
    with_depthpro: bool,
) -> (WorkflowGraph, WorkflowNodeId, Option<WorkflowNodeId>) {
    let g = WorkflowGraph::default();

    let image_node = g.add(ImageScale {
        upscale_method: "nearest-exact",
        width: 1024,
        height: 1024,
        crop: "disabled",
        image: g.add(LoadImage::new(RENDER_FILENAME)).image,
    });

    let preview_image = g.add(PreviewImage::new(image_node));

    let preview_image_depth = with_depthpro.then(|| depthpro_nodes(&g, image_node.into()));

    (g, preview_image, preview_image_depth)
}

fn depthpro_nodes(g: &WorkflowGraph, input: WorkflowInput) -> WorkflowNodeId {
    let load_depthpro = g.add_dynamic(
        WorkflowNode::new("LoadDepthPro")
            .with_input("precision", "fp16")
            .with_meta(WorkflowMeta::new("(Down)Load Depth Pro model")),
    );

    let depthpro = g.add_typed_dynamic::<(ImageOut, UntypedOut, FloatOut)>(
        WorkflowNode::new("DepthPro")
            .with_input("depth_pro_model", load_depthpro.to_input_with_slot(0))
            .with_input("image", input)
            .with_meta(WorkflowMeta::new("Depth Pro")),
    );

    g.add_dynamic(
        WorkflowNode::new("PrismataSave")
            .with_input("images", depthpro.0)
            .with_input("focal_avg", depthpro.2)
            .with_meta(WorkflowMeta::new("Prismata Save")),
    )
}
