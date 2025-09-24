use serde::{Deserialize, Serialize};

pub mod prism {
    use super::*;

    /// A PNG image encoded as Base64.
    pub type EncodedPngImage = String;

    /// An EXR image encoded as Base64.
    pub type EncodedExrImage = String;

    // These are sent over HTTP because they're too big for Lightyear.
    // TODO: Authenticate the user on the server.
    #[derive(Clone, Serialize, Deserialize)]
    pub struct RenderInput {
        pub render_id: u64,
        pub camera_position: [f32; 3],
        pub camera_rotation: [f32; 4],
        pub camera_projection: [f32; 16],
        pub prompt: String,
        pub seed: i64,
        pub repaint_amount: f32,
        pub sampler_steps: u8,
        pub depth_controlnet_strength: f32,
        pub depth_raw: (u32, u32, Vec<f32>),
        pub depth: EncodedPngImage,
        pub near_plane: f32,
        pub far_plane: f32,
        pub depth_min: f32,
        pub depth_max: f32,
        pub base_render: EncodedPngImage,
        pub mask: EncodedPngImage,
    }
    // Event so that it can be sent on the client
    #[derive(Clone, Serialize, Deserialize)]
    pub struct RenderOutput {
        pub render_id: u64,
        pub diffuse: EncodedPngImage,
        /// The metric depth and estimated focal length, if available.
        pub depth: Option<(EncodedExrImage, f32)>,
    }
    #[derive(Clone, Serialize, Deserialize)]
    pub enum RenderOutputResult {
        Ok(RenderOutput),
        Err(String),
    }
    // TODO: find a better home for these + maybe remove image/encoding dependencies
    pub fn encode_image_to_format(
        img: &image::DynamicImage,
        format: image::ImageFormat,
    ) -> image::ImageResult<Vec<u8>> {
        let mut image_data = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut image_data), format)?;
        Ok(image_data)
    }
    pub fn encode_image_to_png(img: &image::DynamicImage) -> image::ImageResult<Vec<u8>> {
        encode_image_to_format(img, image::ImageFormat::Png)
    }
    pub fn encode_image_to_base64(img: &image::DynamicImage) -> image::ImageResult<String> {
        let image_data = encode_image_to_png(img)?;
        Ok(data_encoding::BASE64.encode(&image_data))
    }
    pub fn decode_image_from_base64(data: &str) -> anyhow::Result<image::DynamicImage> {
        let image_data = data_encoding::BASE64.decode(data.as_bytes())?;
        Ok(image::load_from_memory(&image_data)?)
    }
}
