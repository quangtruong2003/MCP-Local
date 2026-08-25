use image::{imageops::FilterType, DynamicImage, ImageFormat, RgbaImage};
use lc_core::error::{CoreError, ErrorCode};
use schemars::JsonSchema;
use serde::Serialize;
use std::io::Cursor;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CaptureTargetKind {
    Display,
    Window,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct CaptureGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct CaptureTarget {
    pub id: String,
    pub kind: CaptureTargetKind,
    pub name: String,
    pub application: Option<String>,
    pub geometry: CaptureGeometry,
    pub scale_factor: f32,
    pub primary: bool,
}

#[derive(Debug, Clone)]
pub struct CapturedImage {
    pub png_bytes: Vec<u8>,
    pub target: CaptureTarget,
    pub original_width: u32,
    pub original_height: u32,
    pub returned_width: u32,
    pub returned_height: u32,
}

pub trait ScreenCaptureProvider: Send + Sync {
    fn list_targets(&self) -> Result<Vec<CaptureTarget>, CoreError>;
    fn capture(&self, target_id: &str, max_edge: Option<u32>) -> Result<CapturedImage, CoreError>;
}

#[derive(Debug, Default)]
pub struct SystemScreenCaptureProvider;

pub fn filter_targets_for_scope(
    targets: Vec<CaptureTarget>,
    scope: crate::settings::CaptureScope,
) -> Vec<CaptureTarget> {
    match scope {
        crate::settings::CaptureScope::WindowsAndDisplays => targets,
        crate::settings::CaptureScope::WindowsOnly => targets
            .into_iter()
            .filter(|target| target.kind == CaptureTargetKind::Window)
            .collect(),
    }
}

pub fn scaled_dimensions(width: u32, height: u32, max_edge: Option<u32>) -> (u32, u32) {
    let Some(max_edge) = max_edge else {
        return (width, height);
    };
    if max_edge == 0 || width == 0 || height == 0 || width.max(height) <= max_edge {
        return (width, height);
    }

    let scale = max_edge as f64 / width.max(height) as f64;
    let scaled_width = ((width as f64 * scale).round() as u32).max(1);
    let scaled_height = ((height as f64 * scale).round() as u32).max(1);
    (scaled_width, scaled_height)
}

fn target_not_found(target_id: &str) -> CoreError {
    CoreError::new(
        ErrorCode::CaptureTargetNotFound,
        format!("capture target '{target_id}' is no longer available"),
    )
}

fn capture_failed(context: &str, error: impl std::fmt::Display) -> CoreError {
    CoreError::new(
        ErrorCode::ScreenCaptureFailed,
        format!("{context}: {error}"),
    )
}

fn encode_png(image: RgbaImage) -> Result<Vec<u8>, CoreError> {
    let mut bytes = Vec::new();
    DynamicImage::ImageRgba8(image)
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .map_err(|e| capture_failed("failed to encode screenshot as PNG", e))?;
    Ok(bytes)
}

fn finish_capture(
    target: CaptureTarget,
    image: RgbaImage,
    max_edge: Option<u32>,
) -> Result<CapturedImage, CoreError> {
    if max_edge == Some(0) {
        return Err(CoreError::new(
            ErrorCode::InvalidArgument,
            "max_edge must be greater than zero",
        ));
    }

    let original_width = image.width();
    let original_height = image.height();
    let (returned_width, returned_height) =
        scaled_dimensions(original_width, original_height, max_edge);
    let returned = if (returned_width, returned_height) == (original_width, original_height) {
        image
    } else {
        image::imageops::resize(
            &image,
            returned_width,
            returned_height,
            FilterType::Lanczos3,
        )
    };
    let png_bytes = encode_png(returned)?;

    Ok(CapturedImage {
        png_bytes,
        target,
        original_width,
        original_height,
        returned_width,
        returned_height,
    })
}

#[cfg(windows)]
fn monitor_target(monitor: &xcap::Monitor) -> Result<CaptureTarget, CoreError> {
    let native_id = monitor
        .id()
        .map_err(|e| capture_failed("failed to read display id", e))?;
    let x = monitor
        .x()
        .map_err(|e| capture_failed("failed to read display x coordinate", e))?;
    let y = monitor
        .y()
        .map_err(|e| capture_failed("failed to read display y coordinate", e))?;
    let width = monitor
        .width()
        .map_err(|e| capture_failed("failed to read display width", e))?;
    let height = monitor
        .height()
        .map_err(|e| capture_failed("failed to read display height", e))?;
    let name = monitor
        .friendly_name()
        .unwrap_or_else(|_| format!("Display {native_id}"));
    let scale_factor = monitor.scale_factor().unwrap_or(1.0);
    let primary = monitor.is_primary().unwrap_or(false);

    Ok(CaptureTarget {
        id: format!("display:{native_id}:{x}:{y}:{width}:{height}"),
        kind: CaptureTargetKind::Display,
        name,
        application: None,
        geometry: CaptureGeometry {
            x,
            y,
            width,
            height,
        },
        scale_factor,
        primary,
    })
}

#[cfg(windows)]
fn window_target(window: &xcap::Window) -> Result<CaptureTarget, CoreError> {
    let native_id = window
        .id()
        .map_err(|e| capture_failed("failed to read window id", e))?;
    let x = window
        .x()
        .map_err(|e| capture_failed("failed to read window x coordinate", e))?;
    let y = window
        .y()
        .map_err(|e| capture_failed("failed to read window y coordinate", e))?;
    let width = window
        .width()
        .map_err(|e| capture_failed("failed to read window width", e))?;
    let height = window
        .height()
        .map_err(|e| capture_failed("failed to read window height", e))?;
    let name = window
        .title()
        .unwrap_or_else(|_| format!("Window {native_id}"));
    let application = window.app_name().ok().filter(|name| !name.is_empty());
    let scale_factor = window
        .current_monitor()
        .ok()
        .and_then(|monitor| monitor.scale_factor().ok())
        .unwrap_or(1.0);

    Ok(CaptureTarget {
        id: format!("window:{native_id}:{x}:{y}:{width}:{height}"),
        kind: CaptureTargetKind::Window,
        name,
        application,
        geometry: CaptureGeometry {
            x,
            y,
            width,
            height,
        },
        scale_factor,
        primary: false,
    })
}

#[cfg(windows)]
impl ScreenCaptureProvider for SystemScreenCaptureProvider {
    fn list_targets(&self) -> Result<Vec<CaptureTarget>, CoreError> {
        let monitors =
            xcap::Monitor::all().map_err(|e| capture_failed("failed to enumerate displays", e))?;
        let windows =
            xcap::Window::all().map_err(|e| capture_failed("failed to enumerate windows", e))?;

        let mut targets = Vec::with_capacity(monitors.len() + windows.len());
        for monitor in monitors {
            targets.push(monitor_target(&monitor)?);
        }
        for window in windows {
            targets.push(window_target(&window)?);
        }
        Ok(targets)
    }

    fn capture(&self, target_id: &str, max_edge: Option<u32>) -> Result<CapturedImage, CoreError> {
        if max_edge == Some(0) {
            return Err(CoreError::new(
                ErrorCode::InvalidArgument,
                "max_edge must be greater than zero",
            ));
        }

        let monitors =
            xcap::Monitor::all().map_err(|e| capture_failed("failed to enumerate displays", e))?;
        for monitor in monitors {
            let target = monitor_target(&monitor)?;
            if target.id == target_id {
                let image = monitor
                    .capture_image()
                    .map_err(|e| capture_failed("failed to capture display", e))?;
                return finish_capture(target, image, max_edge);
            }
        }

        let windows =
            xcap::Window::all().map_err(|e| capture_failed("failed to enumerate windows", e))?;
        for window in windows {
            let target = window_target(&window)?;
            if target.id == target_id {
                let image = window
                    .capture_image()
                    .map_err(|e| capture_failed("failed to capture window", e))?;
                return finish_capture(target, image, max_edge);
            }
        }

        Err(target_not_found(target_id))
    }
}

#[cfg(not(windows))]
impl ScreenCaptureProvider for SystemScreenCaptureProvider {
    fn list_targets(&self) -> Result<Vec<CaptureTarget>, CoreError> {
        Err(CoreError::new(
            ErrorCode::ScreenCaptureUnsupported,
            "screen capture is supported only on Windows in this release",
        ))
    }

    fn capture(
        &self,
        _target_id: &str,
        _max_edge: Option<u32>,
    ) -> Result<CapturedImage, CoreError> {
        Err(CoreError::new(
            ErrorCode::ScreenCaptureUnsupported,
            "screen capture is supported only on Windows in this release",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_target_maps_to_stable_error_code() {
        let error = target_not_found("window:missing");
        assert_eq!(error.code, ErrorCode::CaptureTargetNotFound);
        assert_eq!(error.code.as_str(), "CAPTURE_TARGET_NOT_FOUND");
    }

    #[test]
    fn zero_max_edge_is_rejected_before_resize() {
        let target = CaptureTarget {
            id: "window:1".into(),
            kind: CaptureTargetKind::Window,
            name: "Test".into(),
            application: None,
            geometry: CaptureGeometry {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            scale_factor: 1.0,
            primary: false,
        };
        let error = finish_capture(target, RgbaImage::new(1, 1), Some(0)).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidArgument);
    }
}
