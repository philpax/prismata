use serde::{Deserialize, Serialize};

#[derive(Debug)]
#[allow(dead_code)]
pub enum Error {
    Rucomfyui(rucomfyui::ClientError),
    Image(image::ImageError),
    DataDecoding(data_encoding::DecodeError),
    Utf8(std::str::Utf8Error),
    ParseFloat(std::num::ParseFloatError),
    InvalidImageUpload { filename: String, message: String },
}
impl From<rucomfyui::ClientError> for Error {
    fn from(e: rucomfyui::ClientError) -> Self {
        Self::Rucomfyui(e)
    }
}
impl From<image::ImageError> for Error {
    fn from(e: image::ImageError) -> Self {
        Self::Image(e)
    }
}
impl From<data_encoding::DecodeError> for Error {
    fn from(e: data_encoding::DecodeError) -> Self {
        Self::DataDecoding(e)
    }
}
impl From<std::str::Utf8Error> for Error {
    fn from(e: std::str::Utf8Error) -> Self {
        Self::Utf8(e)
    }
}
impl From<std::num::ParseFloatError> for Error {
    fn from(e: std::num::ParseFloatError) -> Self {
        Self::ParseFloat(e)
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // TODO: improve this
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(default)]
pub struct WorkflowSettings {
    pub dump_workflow: bool,
    pub enable_depthpro: bool,
}
impl Default for WorkflowSettings {
    fn default() -> Self {
        Self {
            dump_workflow: false,
            enable_depthpro: true,
        }
    }
}
