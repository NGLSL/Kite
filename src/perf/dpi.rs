//! 场景：多屏 / DPI 窗口几何。
//! 职责：只压 `system::window_place` 纯计算；真拓扑热插拔标 OS-only。

use super::report::{print_latency, print_section, time_iters, ITERS};
use crate::system::window_place::{center_physical, physical_to_logical_for_window};
use std::hint::black_box;

pub fn run() {
    print_section("DPI / 多屏几何（纯计算，无 Win32 窗口）");

    // (work_l, work_t, work_r, work_b, dpi_scale)
    let monitors = [
        (0.0f32, 0.0f32, 1920.0f32, 1080.0f32, 1.0f32),
        (0.0, 0.0, 2560.0, 1440.0, 1.5),
        (1920.0, 0.0, 4480.0, 1440.0, 1.5),
        (0.0, 0.0, 3840.0, 2160.0, 2.0),
    ];

    let mut finite = true;
    let (p50, p95, max) = time_iters(ITERS, || {
        for &(l, t, r, b, scale) in &monitors {
            let phys = center_physical(l, t, r, b, 640.0, 420.0, scale);
            let logical = physical_to_logical_for_window(phys, scale);
            if !logical.0.is_finite() || !logical.1.is_finite() {
                finite = false;
            }
            black_box((phys, logical));
        }
    });
    print_latency(
        "dpi-center-matrix",
        p50,
        p95,
        max,
        &format!("finite={finite} monitors={}", monitors.len()),
    );
}
