use local_workspace_mcp_manager::{
    screen_capture::{
        filter_targets_for_scope, scaled_dimensions, CaptureGeometry, CaptureTarget,
        CaptureTargetKind,
    },
    settings::CaptureScope,
};

fn target(id: &str, kind: CaptureTargetKind) -> CaptureTarget {
    CaptureTarget {
        id: id.into(),
        kind,
        name: id.into(),
        application: None,
        geometry: CaptureGeometry {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        },
        scale_factor: 1.0,
        primary: false,
    }
}

#[test]
fn scaled_dimensions_downsizes_without_upscaling() {
    assert_eq!(scaled_dimensions(3840, 2160, Some(1920)), (1920, 1080));
    assert_eq!(scaled_dimensions(800, 600, Some(1920)), (800, 600));
    assert_eq!(scaled_dimensions(800, 600, None), (800, 600));
}

#[test]
fn windows_only_scope_filters_full_displays() {
    let targets = vec![
        target("display:1", CaptureTargetKind::Display),
        target("window:7", CaptureTargetKind::Window),
    ];

    let filtered = filter_targets_for_scope(targets.clone(), CaptureScope::WindowsOnly);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].kind, CaptureTargetKind::Window);

    let all = filter_targets_for_scope(targets, CaptureScope::WindowsAndDisplays);
    assert_eq!(all.len(), 2);
}
