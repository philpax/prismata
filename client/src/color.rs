#![allow(dead_code)]

use bevy::{
    color::{Color, ColorToComponents, ColorToPacked, Hsva, LinearRgba},
    math::Vec3,
};
use bevy_egui::egui;

// Color <-> ByteRGB
pub type ByteRGB = [u8; 3];
pub fn bevy_color_to_byte_rgb(color: Color) -> ByteRGB {
    color.to_linear().to_u8_array_no_alpha()
}
pub fn byte_rgb_to_bevy_color(color: ByteRGB) -> Color {
    LinearRgba::from_u8_array_no_alpha(color).into()
}

// Color <-> Vec3
pub fn bevy_color_to_vec3(color: Color) -> Vec3 {
    Vec3::from_array(color.to_linear().to_f32_array_no_alpha())
}
pub fn vec3_to_bevy_color(color: Vec3) -> Color {
    LinearRgba::from_f32_array_no_alpha(color.to_array()).into()
}

// ByteRGB <-> Vec3
pub fn byte_rgb_to_vec3(color: ByteRGB) -> Vec3 {
    Vec3::from_array(color.map(|v| v as f32 / 255.0))
}
pub fn vec3_to_byte_rgb(color: Vec3) -> ByteRGB {
    color.to_array().map(|v| (v * 255.0) as u8)
}

// egui Hsv(a) <-> Color
pub fn bevy_color_to_egui_hsv(bevy_color: Color) -> egui::ecolor::Hsva {
    if bevy_color.to_linear() == LinearRgba::BLACK {
        // HACK: if this is black, return egui's black; something about the conversion doesn't work
        return egui::ecolor::Hsva::new(0.0, 0.0, 0.0, 1.0);
    }
    let hsv = Hsva::from(bevy_color);
    egui::ecolor::Hsva::new(hsv.hue / 360.0, hsv.saturation, hsv.value, hsv.alpha)
}
pub fn egui_hsv_to_bevy_color(egui_color: egui::ecolor::Hsva) -> Color {
    Hsva::from_f32_array([
        egui_color.h * 360.0,
        egui_color.s,
        egui_color.v,
        egui_color.a,
    ])
    .into()
}

// egui Color32 <-> Color
pub fn bevy_color_to_egui_color32(bevy_color: Color) -> egui::Color32 {
    let [r, g, b] = bevy_color_to_byte_rgb(bevy_color);
    egui::Color32::from_rgb(r, g, b)
}
pub fn egui_color32_to_bevy_color(egui_color: egui::Color32) -> Color {
    byte_rgb_to_bevy_color([egui_color.r(), egui_color.g(), egui_color.b()])
}
