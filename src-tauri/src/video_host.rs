#![cfg(windows)]
/**
 * Native child HWND that hosts mpv's video output.
 * Sized over the React video-region div via set_geometry(); its HWND is
 * the one we pass to mpv via the `wid` property.
 *
 * The wndproc handles wheel/middle-drag/double-click and forwards them
 * to the Player as zoom/pan/reset commands. An Arc<Player> is stashed in
 * GWLP_USERDATA so the wndproc can reach the player without globals.
 */
use std::ptr;
use std::sync::{Arc, OnceLock};

use windows::core::{implement, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, POINTL, WPARAM};
use windows::Win32::Graphics::Gdi::{GetStockObject, BLACK_BRUSH, HBRUSH};
use windows::Win32::System::Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::{
    OleInitialize, RegisterDragDrop, CF_HDROP, IDropTarget, IDropTarget_Impl, DROPEFFECT,
    DROPEFFECT_COPY, DROPEFFECT_NONE,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_CONTROL, VK_SHIFT,
};
use windows::Win32::UI::Shell::{DragFinish, DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, GetWindowLongPtrW, MoveWindow, RegisterClassExW,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, HCURSOR,
    HICON, HMENU, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_SHOW, WHEEL_DELTA,
    WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN,
    WM_RBUTTONUP, WNDCLASSEXW, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_NOPARENTNOTIFY,
    WS_VISIBLE,
};

use crate::player::Player;

/// Shared slot for the player reference, set after VideoHost is created (the
/// player needs the HWND from VideoHost, so we wire the link up after the fact).
/// Both the wndproc (HostData) and the IDropTarget hold a clone of this Arc.
type PlayerSlot = Arc<parking_lot::RwLock<Option<Arc<Player>>>>;

const CLASS_NAME: &str = "PlayerSageVideoHost";

static CLASS_REGISTERED: OnceLock<()> = OnceLock::new();

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Per-window state attached via GWLP_USERDATA. Boxed so we have a stable
/// address; dropped when the host window is destroyed (handled in wndproc
/// WM_NCDESTROY — kept simple for now: leak is acceptable since one host
/// is created at app startup).
struct HostData {
    player: PlayerSlot,
    drag: parking_lot::Mutex<Option<DragState>>,
}

struct DragState {
    /// Position where the button went down (client coords).
    anchor: (i32, i32),
    /// Last cursor position seen, used to compute incremental pan deltas.
    last: (i32, i32),
    /// True if Ctrl was held at button-down — gesture is pan, not click.
    pan: bool,
}

const CLICK_THRESHOLD_PX: i32 = 4;

unsafe fn host_data(hwnd: HWND) -> Option<&'static HostData> {
    let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    if p == 0 {
        None
    } else {
        Some(&*(p as *const HostData))
    }
}

fn client_size(hwnd: HWND) -> (i32, i32) {
    let mut r = windows::Win32::Foundation::RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut r);
    }
    (r.right - r.left, r.bottom - r.top)
}

fn loword_signed(l: usize) -> i32 {
    ((l & 0xFFFF) as i16) as i32
}
fn hiword_signed(l: usize) -> i32 {
    (((l >> 16) & 0xFFFF) as i16) as i32
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let data = host_data(hwnd);

    match msg {
        WM_LBUTTONDOWN => {
            if let Some(d) = data {
                let x = loword_signed(l.0 as usize);
                let y = hiword_signed(l.0 as usize);
                *d.drag.lock() = Some(DragState {
                    anchor: (x, y),
                    last: (x, y),
                    pan: false,
                });
                let _ = SetCapture(hwnd);
                let _ = SetFocus(hwnd);
            }
            return LRESULT(0);
        }
        WM_LBUTTONUP => {
            if let Some(d) = data {
                let drag = d.drag.lock().take();
                let _ = ReleaseCapture();
                if let Some(DragState { anchor, last, pan }) = drag {
                    if !pan {
                        let dx = (last.0 - anchor.0).abs();
                        let dy = (last.1 - anchor.1).abs();
                        if dx <= CLICK_THRESHOLD_PX && dy <= CLICK_THRESHOLD_PX {
                            if let Some(p) = d.player.read().as_ref() {
                                let _ = p.toggle_play_pause();
                            }
                        }
                    }
                }
            }
            return LRESULT(0);
        }
        WM_RBUTTONDOWN => {
            if let Some(d) = data {
                let x = loword_signed(l.0 as usize);
                let y = hiword_signed(l.0 as usize);
                *d.drag.lock() = Some(DragState {
                    anchor: (x, y),
                    last: (x, y),
                    pan: true,
                });
                let _ = SetCapture(hwnd);
                let _ = SetFocus(hwnd);
            }
            return LRESULT(0);
        }
        WM_RBUTTONUP => {
            if let Some(d) = data {
                *d.drag.lock() = None;
                let _ = ReleaseCapture();
            }
            return LRESULT(0);
        }
        WM_MOUSEMOVE => {
            if let Some(d) = data {
                let mut drag_guard = d.drag.lock();
                if let Some(state) = drag_guard.as_mut() {
                    let x = loword_signed(l.0 as usize);
                    let y = hiword_signed(l.0 as usize);
                    let (lx, ly) = state.last;
                    state.last = (x, y);
                    if state.pan {
                        let dx = (x - lx) as f64;
                        let dy = (y - ly) as f64;
                        drop(drag_guard);
                        if let Some(p) = d.player.read().as_ref() {
                            p.pan_by_pixels(dx, dy);
                        }
                    }
                }
            }
        }
        WM_KEYDOWN => {
            // VK codes: SPACE=0x20, LEFT=0x25, RIGHT=0x27, '0'=0x30,
            // 'A'=0x41, 'B'=0x42, 'C'=0x43, 'R'=0x52.
            let vk = w.0 as u32;
            if let Some(d) = data {
                let player = d.player.read().as_ref().cloned();
                if let Some(p) = player {
                    match vk {
                        0x20 => {
                            let _ = p.toggle_play_pause();
                            return LRESULT(0);
                        }
                        0x30 | 0x52 => {
                            p.reset_view();
                            return LRESULT(0);
                        }
                        0x41 => {
                            let _ = p.set_loop_a();
                            return LRESULT(0);
                        }
                        0x42 => {
                            let _ = p.set_loop_b();
                            return LRESULT(0);
                        }
                        0x43 => {
                            p.clear_loop();
                            return LRESULT(0);
                        }
                        0x25 | 0x27 => {
                            let ctrl = (GetKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000) != 0;
                            let shift = (GetKeyState(VK_SHIFT.0 as i32) as u16 & 0x8000) != 0;
                            let step = if ctrl { 1.0 } else if shift { 20.0 } else { 5.0 };
                            let delta = if vk == 0x25 { -step } else { step };
                            let _ = p.seek_relative(delta);
                            return LRESULT(0);
                        }
                        _ => {}
                    }
                }
            }
        }
        WM_MOUSEWHEEL => {
            if let Some(d) = data {
                // WM_MOUSEWHEEL coords are SCREEN coords, not client.
                let sx = loword_signed(l.0 as usize);
                let sy = hiword_signed(l.0 as usize);
                let mut pt = POINT { x: sx, y: sy };
                let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut pt);
                let delta = (((w.0 >> 16) & 0xFFFF) as i16) as f64 / WHEEL_DELTA as f64;
                // Ctrl modifier (low word of wParam) → finer zoom steps.
                let ctrl_down = (w.0 & 0x0008) != 0; // MK_CONTROL
                let step = if ctrl_down { 0.05 } else { 0.15 };
                let (cw, ch) = client_size(hwnd);
                if let Some(p) = d.player.read().as_ref() {
                    p.nudge_zoom(delta * step, pt.x as f64, pt.y as f64, cw as f64, ch as f64);
                }
            }
            return LRESULT(0);
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, w, l)
}

fn ensure_class_registered() {
    CLASS_REGISTERED.get_or_init(|| unsafe {
        let class_name = wide(CLASS_NAME);
        let hinstance = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance.into(),
            hIcon: HICON::default(),
            hCursor: HCURSOR::default(),
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            hIconSm: HICON::default(),
        };
        let _atom = RegisterClassExW(&wc);
    });
}

pub struct VideoHost {
    hwnd: HWND,
    data: *const HostData,
    player_slot: PlayerSlot,
    // Kept alive so RegisterDragDrop's reference stays valid (also AddRef'd by OS).
    _drop_target: IDropTarget,
}

impl VideoHost {
    pub fn create(parent: isize) -> Result<Self, String> {
        ensure_class_registered();
        let class_name = wide(CLASS_NAME);
        let title = wide("");
        let parent_hwnd = HWND(parent as *mut _);
        let hwnd = unsafe {
            let hinstance = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
            CreateWindowExW(
                WS_EX_NOPARENTNOTIFY,
                PCWSTR(class_name.as_ptr()),
                PCWSTR(title.as_ptr()),
                WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                0,
                0,
                16,
                16,
                parent_hwnd,
                HMENU::default(),
                hinstance,
                None,
            )
            .map_err(|e| format!("CreateWindowExW failed: {e:?}"))?
        };

        let player_slot: PlayerSlot = Arc::new(parking_lot::RwLock::new(None));

        let data = Box::into_raw(Box::new(HostData {
            player: player_slot.clone(),
            drag: parking_lot::Mutex::new(None),
        })) as *const HostData;

        let drop_target: IDropTarget = VideoDropTarget {
            player: player_slot.clone(),
            hwnd: hwnd.0 as isize,
        }
        .into();

        unsafe {
            // OleInitialize is reference-counted and safe to call multiple times;
            // Tauri's webview already initializes COM/OLE on this thread, but be defensive.
            let _ = OleInitialize(None);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, data as isize);
            RegisterDragDrop(hwnd, &drop_target)
                .map_err(|e| format!("RegisterDragDrop failed: {e:?}"))?;
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetWindowPos(
                hwnd,
                HWND_TOP,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }

        Ok(Self {
            hwnd,
            data,
            player_slot,
            _drop_target: drop_target,
        })
    }

    pub fn hwnd_isize(&self) -> isize {
        self.hwnd.0 as isize
    }

    pub fn focus_self(&self) {
        unsafe {
            let _ = SetFocus(self.hwnd);
        }
    }

    pub fn set_geometry(&self, x: i32, y: i32, width: i32, height: i32) {
        unsafe {
            let _ = MoveWindow(self.hwnd, x, y, width.max(1), height.max(1), true);
            let _ = SetWindowPos(
                self.hwnd,
                HWND_TOP,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }

    pub fn attach_player(&self, player: Arc<Player>) {
        *self.player_slot.write() = Some(player);
    }
}

#[implement(IDropTarget)]
struct VideoDropTarget {
    player: PlayerSlot,
    /// HWND of the video host, used to grab focus after a successful drop
    /// so subsequent keyboard input goes to our wndproc.
    hwnd: isize,
}

#[allow(non_snake_case)]
impl IDropTarget_Impl for VideoDropTarget_Impl {
    fn DragEnter(
        &self,
        pDataObj: Option<&IDataObject>,
        _grfKeyState: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        pdwEffect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        unsafe {
            *pdwEffect = if data_has_files(pDataObj) {
                DROPEFFECT_COPY
            } else {
                DROPEFFECT_NONE
            };
        }
        Ok(())
    }

    fn DragOver(
        &self,
        _grfKeyState: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        pdwEffect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        unsafe {
            if (*pdwEffect).0 == 0 {
                *pdwEffect = DROPEFFECT_COPY;
            }
        }
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        Ok(())
    }

    fn Drop(
        &self,
        pDataObj: Option<&IDataObject>,
        _grfKeyState: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        pdwEffect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        unsafe {
            *pdwEffect = DROPEFFECT_COPY;
            if let Some(path) = first_file_path(pDataObj) {
                if let Some(p) = self.player.read().as_ref() {
                    let _ = p.load(&path);
                    // Grab keyboard focus so arrow keys / space / A-B etc. work
                    // immediately without requiring a separate click.
                    let _ = SetFocus(HWND(self.hwnd as *mut _));
                }
            }
        }
        Ok(())
    }
}

fn drop_format() -> FORMATETC {
    FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

unsafe fn data_has_files(data: Option<&IDataObject>) -> bool {
    let Some(d) = data else { return false };
    d.QueryGetData(&drop_format()).is_ok()
}

unsafe fn first_file_path(data: Option<&IDataObject>) -> Option<String> {
    let d = data?;
    let medium = d.GetData(&drop_format()).ok()?;
    let hdrop = HDROP(medium.u.hGlobal.0 as *mut _);
    // Count is 0xFFFFFFFF probe; pass empty buffer to get item count.
    let mut empty = [0u16; 0];
    let count = DragQueryFileW(hdrop, 0xFFFFFFFF, Some(&mut empty));
    if count == 0 {
        DragFinish(hdrop);
        return None;
    }
    // Probe length of first path.
    let needed = DragQueryFileW(hdrop, 0, None) as usize;
    let mut buf = vec![0u16; needed + 1];
    let n = DragQueryFileW(hdrop, 0, Some(&mut buf)) as usize;
    let path = if n > 0 {
        Some(String::from_utf16_lossy(&buf[..n]))
    } else {
        None
    };
    DragFinish(hdrop);
    path
}

unsafe impl Send for VideoHost {}
unsafe impl Sync for VideoHost {}
