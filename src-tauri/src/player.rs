use std::ffi::CString;
use std::sync::Arc;

use libmpv2::{
    events::{Event, EventContext, PropertyData},
    Format, Mpv,
};
use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// Call mpv_command with a proper argv array.
/// Avoids libmpv2::Mpv::command, which joins args with spaces and passes to
/// mpv_command_string — fatal for paths with spaces.
fn mpv_command_args(mpv: &Mpv, args: &[&str]) -> anyhow::Result<()> {
    let cstrings: Vec<CString> = args
        .iter()
        .map(|s| CString::new(*s))
        .collect::<Result<_, _>>()?;
    let mut ptrs: Vec<*const std::os::raw::c_char> =
        cstrings.iter().map(|c| c.as_ptr()).collect();
    ptrs.push(std::ptr::null());
    let rc = unsafe { libmpv2_sys::mpv_command(mpv.ctx.as_ptr(), ptrs.as_mut_ptr()) };
    if rc < 0 {
        anyhow::bail!("mpv_command {args:?} failed: {rc}");
    }
    Ok(())
}

#[derive(Default, Clone, Serialize)]
pub struct PlayerState {
    pub loaded: bool,
    pub playing: bool,
    pub position: f64,
    pub duration: f64,
    pub path: Option<String>,
    /// Linear zoom factor (1.0 = 100%). UI uses this directly.
    pub zoom: f64,
    pub speed: f64,
    pub loop_a: Option<f64>,
    pub loop_b: Option<f64>,
}

impl PlayerState {
    fn new() -> Self {
        Self {
            zoom: 1.0,
            speed: 1.0,
            ..Default::default()
        }
    }
}

pub struct Player {
    mpv: Arc<Mpv>,
    state: Arc<Mutex<PlayerState>>,
    app: AppHandle,
}

impl Player {
    pub fn new(app: AppHandle, parent_hwnd: isize) -> anyhow::Result<Self> {
        let parent_hwnd_i64 = parent_hwnd as i64;
        let mpv = Mpv::with_initializer(|init| {
            init.set_option("wid", parent_hwnd_i64)?;
            init.set_option("vo", "gpu-next")?;
            init.set_option("hwdec", "auto-safe")?;
            init.set_option("keep-open", "always")?;
            init.set_option("idle", "yes")?;
            init.set_option("force-window", "no")?;
            init.set_option("input-default-bindings", "no")?;
            init.set_option("input-vo-keyboard", "no")?;
            init.set_option("osc", "no")?;
            init.set_option("osd-level", 0i64)?;
            Ok(())
        })
        .map_err(|e| anyhow::anyhow!("mpv init failed: {e:?}"))?;

        let ctx_addr: usize = mpv.ctx.as_ptr() as usize;
        let mpv = Arc::new(mpv);
        let state = Arc::new(Mutex::new(PlayerState::new()));

        let app_for_thread = app.clone();
        std::thread::Builder::new()
            .name("mpv-events".into())
            .spawn({
                let state = state.clone();
                let app = app_for_thread;
                move || {
                    let ctx = std::ptr::NonNull::new(ctx_addr as *mut libmpv2_sys::mpv_handle)
                        .expect("mpv handle is non-null");
                    let mut ev = EventContext::new(ctx);
                    if let Err(e) = ev.disable_deprecated_events() {
                        eprintln!("disable_deprecated_events: {e:?}");
                    }
                    let _ = ev.observe_property("pause", Format::Flag, 0);
                    let _ = ev.observe_property("time-pos", Format::Double, 0);
                    let _ = ev.observe_property("duration", Format::Double, 0);
                    let _ = ev.observe_property("path", Format::String, 0);
                    let _ = ev.observe_property("video-zoom", Format::Double, 0);
                    let _ = ev.observe_property("speed", Format::Double, 0);
                    // ab-loop-a/b can hold "no" (unset) or a number, so observe as String.
                    let _ = ev.observe_property("ab-loop-a", Format::String, 0);
                    let _ = ev.observe_property("ab-loop-b", Format::String, 0);

                    loop {
                        let Some(evt_res) = ev.wait_event(-1.0) else {
                            continue;
                        };
                        match evt_res {
                            Ok(Event::PropertyChange { name, change, .. }) => {
                                let mut s = state.lock();
                                match (name, change) {
                                    ("pause", PropertyData::Flag(paused)) => {
                                        s.playing = !paused;
                                    }
                                    ("time-pos", PropertyData::Double(t)) => {
                                        s.position = t;
                                    }
                                    ("duration", PropertyData::Double(d)) => {
                                        s.duration = d;
                                    }
                                    ("path", PropertyData::Str(p)) => {
                                        s.path = Some(p.to_string());
                                        s.loaded = true;
                                    }
                                    ("video-zoom", PropertyData::Double(z)) => {
                                        s.zoom = (2.0_f64).powf(z);
                                    }
                                    ("speed", PropertyData::Double(sp)) => {
                                        s.speed = sp;
                                    }
                                    ("ab-loop-a", PropertyData::Str(v)) => {
                                        s.loop_a = v.parse::<f64>().ok();
                                    }
                                    ("ab-loop-b", PropertyData::Str(v)) => {
                                        s.loop_b = v.parse::<f64>().ok();
                                    }
                                    _ => {}
                                }
                                let snap = s.clone();
                                drop(s);
                                let _ = app.emit("player-state", snap);
                            }
                            Ok(Event::Shutdown) => break,
                            Ok(_) => {}
                            Err(e) => eprintln!("mpv event error: {e:?}"),
                        }
                    }
                }
            })?;

        Ok(Self { mpv, state, app })
    }

    pub fn load(&self, path: &str) -> anyhow::Result<()> {
        mpv_command_args(&self.mpv, &["loadfile", path])
            .map_err(|e| anyhow::anyhow!("loadfile {path}: {e}"))?;
        self.mpv
            .set_property("pause", false)
            .map_err(|e| anyhow::anyhow!("unpause: {e:?}"))?;
        Ok(())
    }

    pub fn play(&self) -> anyhow::Result<()> {
        self.mpv
            .set_property("pause", false)
            .map_err(|e| anyhow::anyhow!("play: {e:?}"))
    }

    pub fn pause(&self) -> anyhow::Result<()> {
        self.mpv
            .set_property("pause", true)
            .map_err(|e| anyhow::anyhow!("pause: {e:?}"))
    }

    pub fn toggle_play_pause(&self) -> anyhow::Result<()> {
        if !self.snapshot().loaded {
            return Ok(());
        }
        let paused = self.mpv.get_property::<bool>("pause").unwrap_or(false);
        self.mpv
            .set_property("pause", !paused)
            .map_err(|e| anyhow::anyhow!("toggle: {e:?}"))
    }

    pub fn seek(&self, seconds: f64) -> anyhow::Result<()> {
        mpv_command_args(
            &self.mpv,
            &[&"seek".to_string(), &format!("{seconds}"), "absolute"],
        )
        .map_err(|e| anyhow::anyhow!("seek: {e}"))
    }

    pub fn seek_relative(&self, delta: f64) -> anyhow::Result<()> {
        if !self.snapshot().loaded {
            return Ok(());
        }
        mpv_command_args(
            &self.mpv,
            &[&"seek".to_string(), &format!("{delta}"), "relative"],
        )
        .map_err(|e| anyhow::anyhow!("seek_relative: {e}"))
    }

    pub fn snapshot(&self) -> PlayerState {
        self.state.lock().clone()
    }

    fn get_f64(&self, name: &str) -> Option<f64> {
        self.mpv.get_property::<f64>(name).ok()
    }

    fn get_i64(&self, name: &str) -> Option<i64> {
        self.mpv.get_property::<i64>(name).ok()
    }

    /// Adjust zoom by `delta_log2` (e.g. +0.1 ≈ 1.07× per wheel tick),
    /// anchored at client-space pixel (anchor_x, anchor_y) inside a client
    /// region of (client_w, client_h). Pan-x/pan-y are updated so the video
    /// pixel under the anchor stays under the anchor after zoom changes.
    pub fn nudge_zoom(
        &self,
        delta_log2: f64,
        anchor_x: f64,
        anchor_y: f64,
        client_w: f64,
        client_h: f64,
    ) {
        if !self.snapshot().loaded || client_w <= 1.0 || client_h <= 1.0 {
            return;
        }
        let z_old = self.get_f64("video-zoom").unwrap_or(0.0);
        let z_new = (z_old + delta_log2).clamp(-3.0, 6.0);
        let k = (2.0_f64).powf(z_new - z_old);
        if (k - 1.0).abs() < 1e-9 {
            return;
        }

        // dwidth/dheight: the size mpv actually draws the video at (pre-pan).
        let dw = self.get_i64("dwidth").unwrap_or(0) as f64;
        let dh = self.get_i64("dheight").unwrap_or(0) as f64;
        if dw <= 0.0 || dh <= 0.0 {
            // No video info — just apply zoom.
            let _ = self.mpv.set_property("video-zoom", z_new);
            return;
        }

        // mpv pan-x is a fraction of the displayed video width. The displayed
        // video center sits at client_center + (pan * dw, pan * dh).
        let cx = client_w * 0.5;
        let cy = client_h * 0.5;
        let pan_x_old = self.get_f64("video-pan-x").unwrap_or(0.0);
        let pan_y_old = self.get_f64("video-pan-y").unwrap_or(0.0);

        // Video pixel coords under the anchor, in *current* scaled pixels.
        // The anchor's distance from the displayed-video center in scaled px:
        let ax = anchor_x - (cx + pan_x_old * dw);
        let ay = anchor_y - (cy + pan_y_old * dh);

        // After zoom by factor k around mpv's center: distance scales by k.
        // We want the anchor to still hit (ax, ay) in the new scaled coords,
        // so the new pan must shift the center to keep that invariant.
        let dw_new = dw * k;
        let dh_new = dh * k;
        let ax_new = ax * k;
        let ay_new = ay * k;
        let pan_x_new = (anchor_x - cx - ax_new) / dw_new;
        let pan_y_new = (anchor_y - cy - ay_new) / dh_new;

        let _ = self.mpv.set_property("video-zoom", z_new);
        let _ = self.mpv.set_property("video-pan-x", pan_x_new);
        let _ = self.mpv.set_property("video-pan-y", pan_y_new);
    }

    /// Pan by `dx`/`dy` screen pixels (positive = drag content right/down).
    pub fn pan_by_pixels(&self, dx: f64, dy: f64) {
        if !self.snapshot().loaded {
            return;
        }
        let dw = self.get_i64("dwidth").unwrap_or(0) as f64;
        let dh = self.get_i64("dheight").unwrap_or(0) as f64;
        if dw <= 0.0 || dh <= 0.0 {
            return;
        }
        let pan_x = self.get_f64("video-pan-x").unwrap_or(0.0) + dx / dw;
        let pan_y = self.get_f64("video-pan-y").unwrap_or(0.0) + dy / dh;
        let _ = self.mpv.set_property("video-pan-x", pan_x);
        let _ = self.mpv.set_property("video-pan-y", pan_y);
    }

    pub fn set_speed(&self, speed: f64) -> anyhow::Result<()> {
        let clamped = speed.clamp(0.0625, 16.0);
        self.mpv
            .set_property("speed", clamped)
            .map_err(|e| anyhow::anyhow!("set speed: {e:?}"))
    }

    /// Unload the current file and clear loaded state. App remains running.
    pub fn stop(&self) -> anyhow::Result<()> {
        // Clear any A/B loop so a future load starts clean.
        self.clear_loop();
        mpv_command_args(&self.mpv, &["stop"])
            .map_err(|e| anyhow::anyhow!("stop: {e}"))?;
        let snap = {
            let mut s = self.state.lock();
            s.loaded = false;
            s.path = None;
            s.position = 0.0;
            s.duration = 0.0;
            s.loop_a = None;
            s.loop_b = None;
            s.clone()
        };
        let _ = self.app.emit("player-state", snap);
        Ok(())
    }

    pub fn set_loop_a(&self) -> anyhow::Result<()> {
        if !self.snapshot().loaded {
            return Ok(());
        }
        let t = self.get_f64("time-pos").unwrap_or(0.0);
        self.mpv
            .set_property("ab-loop-a", format!("{t}").as_str())
            .map_err(|e| anyhow::anyhow!("set ab-loop-a: {e:?}"))
    }

    pub fn set_loop_b(&self) -> anyhow::Result<()> {
        if !self.snapshot().loaded {
            return Ok(());
        }
        let t = self.get_f64("time-pos").unwrap_or(0.0);
        self.mpv
            .set_property("ab-loop-b", format!("{t}").as_str())
            .map_err(|e| anyhow::anyhow!("set ab-loop-b: {e:?}"))
    }

    pub fn clear_loop(&self) {
        let _ = self.mpv.set_property("ab-loop-a", "no");
        let _ = self.mpv.set_property("ab-loop-b", "no");
    }

    pub fn reset_view(&self) {
        let _ = self.mpv.set_property("video-zoom", 0.0);
        let _ = self.mpv.set_property("video-pan-x", 0.0);
        let _ = self.mpv.set_property("video-pan-y", 0.0);
    }
}
