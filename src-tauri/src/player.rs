use std::ffi::CString;
use std::sync::Arc;

use libmpv2::{
    events::{Event, EventContext, PropertyData},
    Format, Mpv,
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

/**
 * Playback settings that carry over to the next video, also after the
 * app restarts. A-B loop points are not kept: they are times in one video.
 */
#[derive(Serialize, Deserialize)]
struct PlaybackPrefs {
    speed: f64,
    loop_file: bool,
    #[serde(default)]
    play_next: bool,
}

const VIDEO_EXTENSIONS: [&str; 6] = ["mov", "mp4", "m4v", "mkv", "webm", "avi"];

/**
 * Compare two file names the way Windows Explorer does: ignore case, and
 * compare runs of digits as numbers, so "2.mp4" comes before "10.mp4".
 */
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let a: Vec<char> = a.to_lowercase().chars().collect();
    let b: Vec<char> = b.to_lowercase().chars().collect();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let (si, sj) = (i, j);
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let da: String = a[si..i].iter().collect();
            let db: String = b[sj..j].iter().collect();
            let (da, db) = (da.trim_start_matches('0'), db.trim_start_matches('0'));
            let ord = da.len().cmp(&db.len()).then_with(|| da.cmp(db));
            if ord != Ordering::Equal {
                return ord;
            }
        } else {
            if a[i] != b[j] {
                return a[i].cmp(&b[j]);
            }
            i += 1;
            j += 1;
        }
    }
    (a.len() - i).cmp(&(b.len() - j))
}

/**
 * The video that follows `path` in the same folder, in file-name order.
 * Returns None when `path` is the last video in the folder.
 */
fn next_video(path: &str) -> Option<String> {
    let p = std::path::Path::new(path);
    let current = p.file_name()?.to_string_lossy().to_lowercase();
    let dir = match p.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => std::path::PathBuf::from("."),
    };
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| {
            std::path::Path::new(n)
                .extension()
                .map(|x| {
                    let x = x.to_string_lossy().to_lowercase();
                    VIDEO_EXTENSIONS.contains(&x.as_str())
                })
                .unwrap_or(false)
        })
        .collect();
    names.sort_by(|a, b| natural_cmp(a, b));
    let idx = names.iter().position(|n| n.to_lowercase() == current)?;
    let next = names.get(idx + 1)?;
    Some(dir.join(next).to_string_lossy().into_owned())
}

fn prefs_path(app: &AppHandle) -> Option<std::path::PathBuf> {
    Some(app.path().app_config_dir().ok()?.join("playback.json"))
}

fn read_prefs(app: &AppHandle) -> Option<PlaybackPrefs> {
    let text = std::fs::read_to_string(prefs_path(app)?).ok()?;
    serde_json::from_str(&text).ok()
}

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
    /// Whole-file loop (mpv loop-file=inf).
    pub loop_file: bool,
    /// At the end of a video, play the next video in the same folder.
    pub play_next: bool,
    /// Stopped: the file is unloaded (black screen) but `path` is kept so
    /// Play can load it again from the start.
    pub stopped: bool,
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

        let mut initial = PlayerState::new();
        if let Some(prefs) = read_prefs(&app) {
            let _ = mpv.set_property("speed", prefs.speed.clamp(0.0625, 16.0));
            let _ = mpv.set_property("loop-file", if prefs.loop_file { "inf" } else { "no" });
            initial.play_next = prefs.play_next;
        }

        let ctx_addr: usize = mpv.ctx.as_ptr() as usize;
        let mpv = Arc::new(mpv);
        let state = Arc::new(Mutex::new(initial));

        let app_for_thread = app.clone();
        std::thread::Builder::new()
            .name("mpv-events".into())
            .spawn({
                let state = state.clone();
                let app = app_for_thread;
                let mpv = mpv.clone();
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
                    let _ = ev.observe_property("loop-file", Format::String, 0);
                    let _ = ev.observe_property("eof-reached", Format::Flag, 0);

                    loop {
                        let Some(evt_res) = ev.wait_event(-1.0) else {
                            continue;
                        };
                        match evt_res {
                            Ok(Event::PropertyChange { name, change, .. }) => {
                                let mut next: Option<String> = None;
                                let mut s = state.lock();
                                match (name, change) {
                                    ("eof-reached", PropertyData::Flag(true)) => {
                                        if s.play_next {
                                            next = s.path.as_deref().and_then(next_video);
                                        }
                                    }
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
                                    ("loop-file", PropertyData::Str(v)) => {
                                        s.loop_file = v != "no";
                                    }
                                    _ => {}
                                }
                                let snap = s.clone();
                                drop(s);
                                let _ = app.emit("player-state", snap);
                                if let Some(path) = next {
                                    // mpv pauses at the end of a file, so unpause after the load.
                                    if mpv_command_args(&mpv, &["loadfile", &path]).is_ok() {
                                        let _ = mpv.set_property("pause", false);
                                    }
                                }
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
        self.state.lock().stopped = false;
        mpv_command_args(&self.mpv, &["loadfile", path])
            .map_err(|e| anyhow::anyhow!("loadfile {path}: {e}"))?;
        self.mpv
            .set_property("pause", false)
            .map_err(|e| anyhow::anyhow!("unpause: {e:?}"))?;
        Ok(())
    }

    /// With keep-open, mpv pauses on the last frame at end of file and
    /// unpausing there does nothing. Rewind first so Play replays the file.
    fn rewind_if_at_end(&self) {
        if self.mpv.get_property::<bool>("eof-reached").unwrap_or(false) {
            let _ = mpv_command_args(&self.mpv, &["seek", "0", "absolute"]);
        }
    }

    /// If stopped, load the file again. Returns true when it did.
    fn reload_if_stopped(&self) -> anyhow::Result<bool> {
        let snap = self.snapshot();
        match (snap.stopped, snap.path) {
            (true, Some(path)) => self.load(&path).map(|_| true),
            _ => Ok(false),
        }
    }

    pub fn play(&self) -> anyhow::Result<()> {
        if self.reload_if_stopped()? {
            return Ok(());
        }
        self.rewind_if_at_end();
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
        if self.reload_if_stopped()? {
            return Ok(());
        }
        if !self.snapshot().loaded {
            return Ok(());
        }
        let paused = self.mpv.get_property::<bool>("pause").unwrap_or(false);
        if paused {
            self.rewind_if_at_end();
        }
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

    pub fn app_handle(&self) -> AppHandle {
        self.app.clone()
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
            .map_err(|e| anyhow::anyhow!("set speed: {e:?}"))?;
        self.save_prefs();
        Ok(())
    }

    pub fn set_play_next(&self, enabled: bool) {
        let snap = {
            let mut s = self.state.lock();
            s.play_next = enabled;
            s.clone()
        };
        let _ = self.app.emit("player-state", snap);
        self.save_prefs();
    }

    /// Write the current speed, loop and play-next settings to disk.
    fn save_prefs(&self) {
        let prefs = PlaybackPrefs {
            play_next: self.state.lock().play_next,
            speed: self.get_f64("speed").unwrap_or(1.0),
            loop_file: self
                .mpv
                .get_property::<String>("loop-file")
                .map(|v| v != "no")
                .unwrap_or(false),
        };
        let Some(path) = prefs_path(&self.app) else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string(&prefs) {
            let _ = std::fs::write(path, text);
        }
    }

    /// Unload the file so the screen goes black, like a classic player's
    /// Stop. The path is kept so Play loads it again from the start.
    pub fn stop(&self) -> anyhow::Result<()> {
        if !self.snapshot().loaded {
            return Ok(());
        }
        // Pause first so load()'s unpause fires an event that resets `playing`.
        let _ = self.mpv.set_property("pause", true);
        mpv_command_args(&self.mpv, &["stop"])
            .map_err(|e| anyhow::anyhow!("stop: {e}"))?;
        let snap = {
            let mut s = self.state.lock();
            s.loaded = false;
            s.stopped = true;
            s.playing = false;
            s.position = 0.0;
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

    pub fn toggle_loop_file(&self) -> anyhow::Result<()> {
        let on = self.snapshot().loop_file;
        self.mpv
            .set_property("loop-file", if on { "no" } else { "inf" })
            .map_err(|e| anyhow::anyhow!("set loop-file: {e:?}"))?;
        self.save_prefs();
        Ok(())
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
