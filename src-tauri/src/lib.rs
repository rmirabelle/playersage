mod player;
mod updater;
#[cfg(windows)]
mod video_host;

use std::sync::Arc;

use parking_lot::Mutex;
use tauri::{
    menu::{Menu, MenuItem, Submenu},
    Emitter, Manager, State,
};

use player::{Player, PlayerState};

#[cfg(windows)]
use video_host::VideoHost;

struct AppState {
    player: Mutex<Option<Arc<Player>>>,
    #[cfg(windows)]
    host: Mutex<Option<Arc<VideoHost>>>,
}

#[tauri::command]
fn load_file(path: String, state: State<'_, AppState>) -> Result<(), String> {
    let player = state.player.lock().clone().ok_or("player not ready")?;
    player.load(&path).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        if let Some(host) = state.host.lock().as_ref() {
            host.focus_self();
        }
    }
    Ok(())
}

#[tauri::command]
fn play(state: State<'_, AppState>) -> Result<(), String> {
    let player = state.player.lock().clone().ok_or("player not ready")?;
    player.play().map_err(|e| e.to_string())
}

#[tauri::command]
fn pause(state: State<'_, AppState>) -> Result<(), String> {
    let player = state.player.lock().clone().ok_or("player not ready")?;
    player.pause().map_err(|e| e.to_string())
}

#[tauri::command]
fn seek(seconds: f64, state: State<'_, AppState>) -> Result<(), String> {
    let player = state.player.lock().clone().ok_or("player not ready")?;
    player.seek(seconds).map_err(|e| e.to_string())
}

#[tauri::command]
fn reset_view(state: State<'_, AppState>) -> Result<(), String> {
    let player = state.player.lock().clone().ok_or("player not ready")?;
    player.reset_view();
    Ok(())
}

#[tauri::command]
fn toggle_play_pause(state: State<'_, AppState>) -> Result<(), String> {
    let player = state.player.lock().clone().ok_or("player not ready")?;
    player.toggle_play_pause().map_err(|e| e.to_string())
}

#[tauri::command]
fn set_speed(speed: f64, state: State<'_, AppState>) -> Result<(), String> {
    let player = state.player.lock().clone().ok_or("player not ready")?;
    player.set_speed(speed).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_loop_a(state: State<'_, AppState>) -> Result<(), String> {
    let player = state.player.lock().clone().ok_or("player not ready")?;
    player.set_loop_a().map_err(|e| e.to_string())
}

#[tauri::command]
fn set_loop_b(state: State<'_, AppState>) -> Result<(), String> {
    let player = state.player.lock().clone().ok_or("player not ready")?;
    player.set_loop_b().map_err(|e| e.to_string())
}

#[tauri::command]
fn clear_loop(state: State<'_, AppState>) -> Result<(), String> {
    let player = state.player.lock().clone().ok_or("player not ready")?;
    player.clear_loop();
    Ok(())
}

#[tauri::command]
fn stop(state: State<'_, AppState>) -> Result<(), String> {
    let player = state.player.lock().clone().ok_or("player not ready")?;
    player.stop().map_err(|e| e.to_string())
}

#[tauri::command]
fn seek_relative(delta: f64, state: State<'_, AppState>) -> Result<(), String> {
    let player = state.player.lock().clone().ok_or("player not ready")?;
    player.seek_relative(delta).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_state(state: State<'_, AppState>) -> PlayerState {
    state
        .player
        .lock()
        .as_ref()
        .map(|p| p.snapshot())
        .unwrap_or_default()
}

#[cfg(windows)]
#[tauri::command]
fn set_video_region(
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if let Some(host) = state.host.lock().as_ref() {
        host.set_geometry(x, y, width, height);
    }
    Ok(())
}

#[cfg(not(windows))]
#[tauri::command]
fn set_video_region(
    _x: i32,
    _y: i32,
    _width: i32,
    _height: i32,
    _state: State<'_, AppState>,
) -> Result<(), String> {
    Err("PlayerSage currently supports Windows only.".into())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .manage(AppState {
            player: Mutex::new(None),
            #[cfg(windows)]
            host: Mutex::new(None),
        })
        .setup(|app| {
            // Help → About menu
            let handle = app.handle();
            let about = MenuItem::with_id(handle, "about", "About Player Sage", true, None::<&str>)?;
            let help = Submenu::with_items(handle, "Help", true, &[&about])?;
            let menu = Menu::with_items(handle, &[&help])?;
            app.set_menu(menu)?;
            app.on_menu_event(|app, event| {
                if event.id().as_ref() == "about" {
                    let _ = app.emit("show-about", ());
                }
            });
            #[cfg(windows)]
            {
                let main = app
                    .get_webview_window("main")
                    .ok_or("no main window")?;
                let parent_hwnd = main.hwnd()?.0 as isize;

                let host = Arc::new(VideoHost::create(parent_hwnd).map_err(|e| {
                    Box::<dyn std::error::Error>::from(format!("video host: {e}"))
                })?);
                let host_hwnd = host.hwnd_isize();

                let player = Arc::new(
                    Player::new(app.handle().clone(), host_hwnd)
                        .map_err(|e| Box::<dyn std::error::Error>::from(format!("player: {e}")))?,
                );

                host.attach_player(player.clone());

                let state: State<'_, AppState> = app.state();
                *state.host.lock() = Some(host);
                *state.player.lock() = Some(player);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_file,
            play,
            pause,
            seek,
            reset_view,
            toggle_play_pause,
            set_speed,
            set_loop_a,
            set_loop_b,
            clear_loop,
            stop,
            seek_relative,
            get_state,
            set_video_region,
            updater::check_for_update,
            updater::download_and_run_installer,
            updater::get_app_version
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
