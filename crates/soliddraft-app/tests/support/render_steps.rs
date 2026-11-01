//! When a rendered app test draws. A step runs the real UI on the real input; a draw adds a
//! GPU pass and a copy of the whole window. So a case steps the UI alone while it settles
//! or works the chrome, and draws where it reads pixels, moves through the viewport or
//! needs the id buffer to answer. Nothing here changes what a step does or how many a case
//! takes, and no case asserts how long anything took.
//!
//! A support module or test file brings this in with `#[path]`, once per test binary.

#![allow(dead_code)]

use egui::Pos2;
use egui_kittest::Harness;
use egui_kittest::wgpu::{WgpuTestRenderer, create_render_state, default_wgpu_setup};
use soliddraft_app::app::SolidDraftApp;
use soliddraft_app::viewport::planes::sample_is_current;
use soliddraft_app::viewport::render::PickSample;
use std::io::Write;
use std::sync::Once;
use std::time::{Duration, Instant};

type App<'a> = Harness<'a, SolidDraftApp>;

/// The ten-minute guard on every wait for something to happen.
pub const HANG_GUARD: Duration = Duration::from_secs(600);

/// The same renderer as `HarnessBuilder::wgpu`, naming the adapter once per test binary. It
/// writes to stderr itself because libtest captures `eprintln!` from a test that passes.
pub fn renderer() -> WgpuTestRenderer {
    let state = create_render_state(default_wgpu_setup(), egui_wgpu::RendererOptions::PREDICTABLE);
    static NAMED: Once = Once::new();
    NAMED.call_once(|| {
        let _ = writeln!(std::io::stderr(), "selected adapter {:?}", state.adapter.get_info());
    });
    WgpuTestRenderer::from_render_state(state)
}

/// Where a harness keeps whether it draws every step: it has a setter and no getter, and
/// the context is the one thing every copy of this module shares for a harness.
fn eager_id() -> egui::Id {
    egui::Id::new("render_steps eager")
}

fn eager(app: &App<'_>) -> bool {
    app.ctx.data(|d| d.get_temp(eager_id())).unwrap_or(false)
}

/// Makes every step of a harness draw, or only those a [`rendered`] scope asks for. A case
/// that must stay eager says so here, and a scope inside it leaves it eager.
pub fn set_eager(app: &mut App<'_>, on: bool) {
    app.ctx.data_mut(|d| d.insert_temp(eager_id(), on));
    app.set_render_every_step(on);
}

/// Draws every step of `scope`, and leaves the harness as the caller had it.
pub fn rendered<'a, R>(app: &mut App<'a>, scope: impl FnOnce(&mut App<'a>) -> R) -> R {
    let caller = eager(app);
    set_eager(app, true);
    let result = scope(app);
    set_eager(app, caller);
    result
}

/// One step of the UI and one draw of what it painted.
pub fn gpu_step(app: &mut App<'_>) {
    rendered(app, |app| app.step());
}

fn sample(app: &App<'_>) -> PickSample {
    app.state().state.viewport.pick_sample()
}

/// Whether the id buffer shows the pixel under `point` of the frame now on screen, from a
/// readback after `floor`.
fn pick_is_current(app: &App<'_>, point: Pos2, floor: u64) -> bool {
    let state = &app.state().state;
    let (doc, viewport) = (&state.doc, &state.viewport);
    let (rect, ppp) = (viewport.rect, app.ctx.pixels_per_point());
    let px = [((point.x - rect.left()) * ppp).max(0.0) as u32, ((point.y - rect.top()) * ppp).max(0.0) as u32];
    let revision = Some((doc.model().id(), doc.mesh_version, doc.display_body()));
    let sample = sample(app);
    sample.serial > floor && sample_is_current(&sample, revision, viewport.pick_view(rect, ppp), px)
}

/// Moves the pointer to a point of the viewport, draws until the id buffer answers for that
/// pixel of this frame, and lets the UI read it. Returns the steps it took.
///
/// A serial newer than the one on entry would not do: a readback from before the wait can
/// still publish, and a display choice need not change the revision it carries. Every draw
/// waits for the GPU, so the first one here publishes whatever was in flight, and the
/// floor it leaves can only be passed by a draw of this wait.
///
/// Only for a viewport that reads ids. A sketch issues none, and a control over the view
/// takes the pointer from it, so either would wait out the guard.
pub fn await_pick(app: &mut App<'_>, point: Pos2) -> usize {
    rendered(app, |app| {
        app.hover_at(point);
        app.step();
        let floor = sample(app).serial;
        let mut steps = 1;
        let start = Instant::now();
        while !pick_is_current(app, point, floor) {
            assert!(start.elapsed() < HANG_GUARD, "no current id-buffer sample at {point:?}");
            app.step();
            steps += 1;
        }
        app.step();
        steps + 1
    })
}
