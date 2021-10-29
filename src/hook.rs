use std::ffi::c_void;
use std::ffi::CString;
use std::ptr::null_mut;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::sync::RwLock;

use imgui::NavInput;
use lazy_static::lazy_static;
use log::*;
use once_cell::sync::OnceCell;

use winapi::shared::dxgi::*;
use winapi::shared::dxgiformat::*;
use winapi::shared::dxgitype::*;
use winapi::shared::minwindef::*;
use winapi::shared::windef::{HBRUSH, HICON, HMENU, HWND, POINT, RECT};
use winapi::um::d3d11::*;
use winapi::um::d3dcommon::*;
use winapi::um::libloaderapi::GetProcAddress;
use winapi::um::libloaderapi::LoadLibraryA;
use winapi::um::winnt::*;
use winapi::um::winuser::*;
use winapi::um::xinput::*;
use winapi::Interface;

use crate::imgui_impl;
use crate::mh;

type XInputGetStateType =
  unsafe extern "system" fn(dw_user_index: DWORD, p_state: *mut XINPUT_STATE) -> DWORD;

type DXGISwapChainPresentType =
  unsafe extern "system" fn(This: *mut IDXGISwapChain, SyncInterval: UINT, Flags: UINT) -> HRESULT;

type WndProcType =
  unsafe extern "system" fn(hwnd: HWND, umsg: UINT, wparam: WPARAM, lparam: LPARAM) -> isize;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Data structures and traits
////////////////////////////////////////////////////////////////////////////////////////////////////

struct DxgiHookState {
  dev: *mut ID3D11Device,
  ctx: *mut ID3D11DeviceContext,
  render_target: *mut ID3D11RenderTargetView,
  imgui_renderer: imgui_impl::dx11::Renderer,
  imgui_ctx: imgui::Context,
  controller_state: ControllerState,
  default_wnd_proc: WndProcType,
}
unsafe impl Send for DxgiHookState {}
unsafe impl Sync for DxgiHookState {}

pub struct RenderContext<'a> {
  pub frame: &'a imgui::Ui<'a>,
  pub controller: ControllerState,
}

pub trait RenderLoop: Send + Sync {
  /// Invoked once per frame. Memory management and UI visualization (via the
  /// current frame's `imgui::Ui` instance) should be made inside of it.
  fn render(&mut self, ctx: RenderContext<'_>);
  fn is_visible(&self) -> bool;
  fn is_capturing(&self) -> bool;
}

#[derive(Clone)]
pub struct ControllerState {
  pub up: bool,
  pub down: bool,
  pub left: bool,
  pub right: bool,
  pub start: bool,
  pub back: bool,
  pub l3: bool,
  pub r3: bool,
  pub lb: bool,
  pub rb: bool,
  pub a: bool,
  pub b: bool,
  pub x: bool,
  pub y: bool,

  pub left_stick_x: i16,
  pub left_stick_y: i16,
  pub right_stick_x: i16,
  pub right_stick_y: i16,

  pub lt: u8,
  pub rt: u8,
}

impl Default for ControllerState {
  fn default() -> Self {
    unsafe { std::mem::zeroed() }
  }
}

impl From<&XINPUT_STATE> for ControllerState {
  fn from(i: &XINPUT_STATE) -> Self {
    let g = i.Gamepad;
    ControllerState {
      up: g.wButtons & XINPUT_GAMEPAD_DPAD_UP != 0,
      down: g.wButtons & XINPUT_GAMEPAD_DPAD_DOWN != 0,
      left: g.wButtons & XINPUT_GAMEPAD_DPAD_LEFT != 0,
      right: g.wButtons & XINPUT_GAMEPAD_DPAD_RIGHT != 0,
      start: g.wButtons & XINPUT_GAMEPAD_START != 0,
      back: g.wButtons & XINPUT_GAMEPAD_BACK != 0,
      l3: g.wButtons & XINPUT_GAMEPAD_LEFT_THUMB != 0,
      r3: g.wButtons & XINPUT_GAMEPAD_RIGHT_THUMB != 0,
      lb: g.wButtons & XINPUT_GAMEPAD_LEFT_SHOULDER != 0,
      rb: g.wButtons & XINPUT_GAMEPAD_RIGHT_SHOULDER != 0,
      a: g.wButtons & XINPUT_GAMEPAD_A != 0,
      b: g.wButtons & XINPUT_GAMEPAD_B != 0,
      x: g.wButtons & XINPUT_GAMEPAD_X != 0,
      y: g.wButtons & XINPUT_GAMEPAD_Y != 0,
      left_stick_x: g.sThumbLX,
      left_stick_y: g.sThumbLY,
      right_stick_x: g.sThumbRX,
      right_stick_y: g.sThumbRY,
      lt: g.bLeftTrigger,
      rt: g.bRightTrigger,
    }
  }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Global singletons
////////////////////////////////////////////////////////////////////////////////////////////////////

static DXGI_SWAP_CHAIN_TRAMPOLINE: OnceCell<DXGISwapChainPresentType> = OnceCell::new();
static XINPUT_GET_STATE_TRAMPOLINE: OnceCell<XInputGetStateType> = OnceCell::new();

static DXGI_HOOK_STATE: OnceCell<Mutex<DxgiHookState>> = OnceCell::new();

lazy_static! {
  static ref RENDER_LOOP: OnceCell<Mutex<Box<dyn RenderLoop>>> = OnceCell::new();
  static ref CONTROLLER_STATE: OnceCell<RwLock<ControllerState>> = OnceCell::new();
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Hook entry points
////////////////////////////////////////////////////////////////////////////////////////////////////

unsafe extern "system" fn dxgi_swap_chain_present_impl(
  p_this: *mut IDXGISwapChain,
  sync_interval: UINT,
  flags: UINT,
) -> HRESULT {
  let trampoline = DXGI_SWAP_CHAIN_TRAMPOLINE
    .get()
    .expect("IDXGISwapChain::Present trampoline uninitialized");

  let this = &*p_this;

  let dxgi_hook_state = &mut *DXGI_HOOK_STATE
    .get_or_init(|| {
      let mut dev: *mut ID3D11Device = null_mut();
      let mut ctx: *mut ID3D11DeviceContext = null_mut();
      let mut sd: DXGI_SWAP_CHAIN_DESC = std::mem::zeroed();
      let mut back_buf: *mut ID3D11Texture2D = null_mut();
      let mut render_target: *mut ID3D11RenderTargetView = null_mut();

      let mut lpc: UINT = 0;
      this.GetLastPresentCount(&mut lpc);

      let result = this.GetDevice(
        &ID3D11Device::uuidof(),
        &mut dev as *mut *mut ID3D11Device as *mut *mut c_void,
      );
      if result < 0 {
        panic!(
          "Get device + ctx from swap chain failed: {:x} {:p}",
          result, p_this
        );
      }
      (*dev).GetImmediateContext(&mut ctx as _);
      this.GetDesc(&mut sd as _);

      let mut imgui_ctx = imgui::Context::create();
      imgui_ctx.set_ini_filename(None);
      imgui_ctx.io_mut().nav_active = true;
      imgui_ctx.io_mut().nav_visible = true;
      let imgui_renderer =
        imgui_impl::dx11::Renderer::new(dev, ctx, &mut imgui_ctx).expect("Renderer::new");

      this.GetBuffer(
        0,
        &ID3D11Texture2D::uuidof(),
        &mut back_buf as *mut *mut ID3D11Texture2D as *mut *mut c_void,
      );

      (*dev).CreateRenderTargetView(back_buf as _, null_mut(), &mut render_target);
      (*back_buf).Release();

      #[allow(clippy::fn_to_numeric_cast)]
      let default_wnd_proc = std::mem::transmute(SetWindowLongPtrA(
        sd.OutputWindow,
        GWLP_WNDPROC,
        wnd_proc as _,
      ));

      let controller_state = ControllerState::default();

      Mutex::new(DxgiHookState {
        dev,
        ctx,
        render_target,
        imgui_renderer,
        imgui_ctx,
        controller_state,
        default_wnd_proc,
      })
    })
    .lock()
    .expect("Poisoned DxgiHookState mutex");

  let mut sd: DXGI_SWAP_CHAIN_DESC = std::mem::zeroed();
  let mut rect: RECT = std::mem::zeroed();

  (*dxgi_hook_state.ctx).OMSetRenderTargets(1, &mut dxgi_hook_state.render_target, null_mut());
  this.GetDesc(&mut sd as _);

  if GetWindowRect(sd.OutputWindow, &mut rect as _) != 0 {
    {
      let mut io = dxgi_hook_state.imgui_ctx.io_mut();

      io.display_size = [
        (rect.right - rect.left) as f32,
        (rect.bottom - rect.top) as f32,
      ];

      let mut pos = POINT { x: 0, y: 0 };

      let active_window = GetForegroundWindow();
      if active_window != 0 as HWND
        && (active_window == sd.OutputWindow || IsChild(active_window, sd.OutputWindow) != 0)
      {
        let gcp = GetCursorPos(&mut pos as *mut _);
        if gcp != 0 && ScreenToClient(sd.OutputWindow, &mut pos as *mut _) != 0 {
          io.mouse_pos[0] = pos.x as _;
          io.mouse_pos[1] = pos.y as _;
        }
      }
    }

    let ui = dxgi_hook_state.imgui_ctx.frame();

    let controller = {
      CONTROLLER_STATE
        .get_or_init(|| RwLock::new(ControllerState::default()))
        .read()
        .unwrap()
        .clone()
    };

    {
      RENDER_LOOP
        .get()
        .unwrap()
        .lock()
        .unwrap()
        .render(RenderContext {
          frame: &ui,
          controller,
        });
    }
    let dd = ui.render();

    match dxgi_hook_state.imgui_renderer.render(dd) {
      Ok(_) => {}
      Err(e) => error!("Renderer errored: {:?}", e),
    };
  }

  trampoline(p_this, sync_interval, flags)
}

extern "system" fn xinput_get_state_impl(
  dw_user_index: DWORD,
  p_state: *mut XINPUT_STATE,
) -> DWORD {
  static LAST_PACKET_NUMBER: OnceCell<AtomicU32> = OnceCell::new();

  let mut state: XINPUT_STATE = unsafe { std::mem::zeroed() };
  let retval = unsafe { XInputGetState(dw_user_index, &mut state as *mut _) };

  let is_capturing = { RENDER_LOOP.get().unwrap().lock().unwrap().is_capturing() };

  let lpn = LAST_PACKET_NUMBER.get_or_init(|| AtomicU32::new(0));

  if state.dwPacketNumber != lpn.load(Ordering::Relaxed) {
    let cs = CONTROLLER_STATE.get_or_init(|| RwLock::new(ControllerState::default()));
    let mut cs_mut = cs.write().unwrap();
    *cs_mut = ControllerState::from(&state);

    if let Some(hook) = DXGI_HOOK_STATE.get() {
      let mut hook = hook.lock().unwrap();
      hook.controller_state = cs_mut.clone();
      // let nav_inputs = &mut hook.imgui_ctx.io_mut().nav_inputs;

      // let mut map_button = |i: NavInput, state: bool| {
      //   nav_inputs[i as usize] = if state { 1.0 } else { 0.0 };
      // };

      // map_button(NavInput::Activate, cs_mut.a);
      // map_button(NavInput::Cancel, cs_mut.b);
      // map_button(NavInput::Menu, cs_mut.x);
      // map_button(NavInput::Input, cs_mut.y);
      // map_button(NavInput::DpadLeft, cs_mut.left);
      // map_button(NavInput::DpadRight, cs_mut.right);
      // map_button(NavInput::DpadDown, cs_mut.down);
      // map_button(NavInput::DpadUp, cs_mut.up);
      // map_button(NavInput::FocusNext, cs_mut.rb);
      // map_button(NavInput::FocusPrev, cs_mut.lb);
      // map_button(NavInput::TweakSlow, cs_mut.lb);
      // map_button(NavInput::TweakFast, cs_mut.rb);

      // let mut map_analog = |i: NavInput, state: i16, min: i16, max: i16| {
      //   let vn = (state - min) as f32 / (max - min) as f32;
      //   let vn = f32::min(1.0, vn);
      //   if vn > 0.0 && nav_inputs[i as usize] < vn {
      //     nav_inputs[i as usize] = vn;
      //   }
      // };

      // let l_dz = XINPUT_GAMEPAD_LEFT_THUMB_DEADZONE;

      // map_analog(NavInput::LStickLeft, cs_mut.left_stick_x, -l_dz, -32768);
      // map_analog(NavInput::LStickRight, cs_mut.left_stick_x, l_dz, 32767);
      // map_analog(NavInput::LStickDown, cs_mut.left_stick_y, -l_dz, -32768);
      // map_analog(NavInput::LStickUp, cs_mut.left_stick_y, l_dz, 32767);
    }
  }

  if let Some(m) = unsafe { p_state.as_mut() } {
    if !is_capturing {
      *m = state;
    }
  }

  retval
}

unsafe extern "system" fn wnd_proc(
  hwnd: HWND,
  umsg: UINT,
  wparam: WPARAM,
  lparam: LPARAM,
) -> isize {
  if let Some(hook) = DXGI_HOOK_STATE.get() {
    let mut hook = hook.lock().unwrap();

    let set_capture = |mouse_down: &[bool], hwnd| {
      let any_down = mouse_down.iter().any(|i| *i);
      if !any_down && GetCapture() == 0 as HWND {
        SetCapture(hwnd);
      }
    };

    let release_capture = |mouse_down: &[bool], hwnd| {
      let any_down = mouse_down.iter().any(|i| *i);
      if !any_down && GetCapture() == hwnd {
        ReleaseCapture();
      }
    };

    match umsg {
      WM_KEYDOWN | WM_SYSKEYDOWN => {
        if wparam < 256 {
          hook.imgui_ctx.io_mut().keys_down[wparam] = true;
        }
      }
      WM_KEYUP | WM_SYSKEYUP => {
        if wparam < 256 {
          hook.imgui_ctx.io_mut().keys_down[wparam] = false;
        }
      }
      WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
        // set_capture(&hook.imgui_ctx.io().mouse_down, hwnd);
        hook.imgui_ctx.io_mut().mouse_down[0] = true;
        // return 1;
      }
      WM_RBUTTONDOWN | WM_RBUTTONDBLCLK => {
        // set_capture(&hook.imgui_ctx.io().mouse_down, hwnd);
        hook.imgui_ctx.io_mut().mouse_down[1] = true;
        // return 1;
      }
      WM_MBUTTONDOWN | WM_MBUTTONDBLCLK => {
        // set_capture(&hook.imgui_ctx.io().mouse_down, hwnd);
        hook.imgui_ctx.io_mut().mouse_down[2] = true;
        // return 1;
      }
      WM_XBUTTONDOWN | WM_XBUTTONDBLCLK => {
        let btn = if GET_XBUTTON_WPARAM(wparam) == XBUTTON1 {
          3
        } else {
          4
        };
        // set_capture(&hook.imgui_ctx.io().mouse_down, hwnd);
        hook.imgui_ctx.io_mut().mouse_down[btn] = true;
        // return 1;
      }
      WM_LBUTTONUP => {
        hook.imgui_ctx.io_mut().mouse_down[0] = false;
        // release_capture(&hook.imgui_ctx.io().mouse_down, hwnd);
        // return 1;
      }
      WM_RBUTTONUP => {
        hook.imgui_ctx.io_mut().mouse_down[1] = false;
        // release_capture(&hook.imgui_ctx.io().mouse_down, hwnd);
        // return 1;
      }
      WM_MBUTTONUP => {
        hook.imgui_ctx.io_mut().mouse_down[2] = false;
        // release_capture(&hook.imgui_ctx.io().mouse_down, hwnd);
        // return 1;
      }
      WM_XBUTTONUP => {
        let btn = if GET_XBUTTON_WPARAM(wparam) == XBUTTON1 {
          3
        } else {
          4
        };
        hook.imgui_ctx.io_mut().mouse_down[btn] = false;
        // release_capture(&hook.imgui_ctx.io().mouse_down, hwnd);
      }
      WM_MOUSEWHEEL => {
        hook.imgui_ctx.io_mut().mouse_wheel +=
          (GET_WHEEL_DELTA_WPARAM(wparam) as f32) / (WHEEL_DELTA as f32);
      }
      WM_MOUSEHWHEEL => {
        hook.imgui_ctx.io_mut().mouse_wheel_h +=
          (GET_WHEEL_DELTA_WPARAM(wparam) as f32) / (WHEEL_DELTA as f32);
      }
      WM_CHAR => hook
        .imgui_ctx
        .io_mut()
        .add_input_character(wparam as u8 as char),
      _ => {}
    }

    let wnd_proc = hook.default_wnd_proc;
    drop(hook);

    CallWindowProcW(Some(wnd_proc), hwnd, umsg, wparam, lparam)
  } else {
    debug!("WndProc called before hook was set");
    0
  }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Function address finders
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Get the `IDXGISwapChain::Present` function address.
///
/// Creates a swap chain + device instance and looks up its
/// vtable to find the address.
fn get_present_addr() -> LPVOID {
  let hwnd = {
    let hinstance = unsafe { winapi::um::libloaderapi::GetModuleHandleA(std::ptr::null::<i8>()) };
    let wnd_class = WNDCLASSA {
      style: CS_OWNDC | CS_HREDRAW | CS_VREDRAW,
      lpfnWndProc: Some(DefWindowProcA),
      hInstance: hinstance,
      lpszClassName: "HUDHOOK_DUMMY\0".as_ptr() as *const i8,
      cbClsExtra: 0,
      cbWndExtra: 0,
      hIcon: 0 as HICON,
      hCursor: 0 as HICON,
      hbrBackground: 0 as HBRUSH,
      lpszMenuName: std::ptr::null::<i8>(),
    };
    unsafe {
      RegisterClassA(&wnd_class);
      CreateWindowExA(
        0,
        "HUDHOOK_DUMMY\0".as_ptr() as _,
        "HUDHOOK_DUMMY\0".as_ptr() as _,
        WS_OVERLAPPEDWINDOW | WS_VISIBLE,
        0,
        0,
        16,
        16,
        0 as HWND,
        0 as HMENU,
        std::mem::transmute(hinstance),
        0 as LPVOID,
      )
    }
  };

  let mut feature_level = D3D_FEATURE_LEVEL_11_0;
  let mut swap_chain_desc: DXGI_SWAP_CHAIN_DESC = unsafe { std::mem::zeroed() };
  let mut p_device: *mut ID3D11Device = null_mut();
  let mut p_context: *mut ID3D11DeviceContext = null_mut();
  let mut p_swap_chain: *mut IDXGISwapChain = null_mut();

  swap_chain_desc.BufferCount = 1;
  swap_chain_desc.BufferDesc.Format = DXGI_FORMAT_R8G8B8A8_UNORM;
  swap_chain_desc.BufferDesc.ScanlineOrdering = DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED;
  swap_chain_desc.BufferDesc.Scaling = DXGI_MODE_SCALING_UNSPECIFIED;
  swap_chain_desc.SwapEffect = DXGI_SWAP_EFFECT_DISCARD;
  swap_chain_desc.BufferUsage = DXGI_USAGE_RENDER_TARGET_OUTPUT;
  swap_chain_desc.OutputWindow = hwnd;
  swap_chain_desc.SampleDesc.Count = 1;
  swap_chain_desc.Windowed = 1;

  let result = unsafe {
    D3D11CreateDeviceAndSwapChain(
      std::ptr::null_mut::<IDXGIAdapter>(),
      D3D_DRIVER_TYPE_HARDWARE,
      0 as HMODULE,
      0u32,
      &mut feature_level as *mut D3D_FEATURE_LEVEL,
      1,
      D3D11_SDK_VERSION,
      &mut swap_chain_desc as *mut DXGI_SWAP_CHAIN_DESC,
      &mut p_swap_chain as *mut *mut IDXGISwapChain,
      &mut p_device as *mut *mut ID3D11Device,
      null_mut(),
      &mut p_context as *mut *mut ID3D11DeviceContext,
    )
  };

  if result < 0 {
    panic!("D3D11CreateDeviceAndSwapChain failed {:x}", result);
  }

  let ret = unsafe { (*(*p_swap_chain).lpVtbl).Present };

  unsafe {
    (*p_device).Release();
    (*p_context).Release();
    (*p_swap_chain).Release();
    DestroyWindow(hwnd);
  }

  ret as LPVOID
}

unsafe fn get_xinput_addr() -> LPVOID {
  let xinput_dll = LoadLibraryA(CString::new("xinput1_3.dll").unwrap().as_c_str().as_ptr());
  let xinput_addr = GetProcAddress(
    xinput_dll,
    CString::new("XInputGetState").unwrap().as_c_str().as_ptr(),
  );
  xinput_addr as _
}

/// Inner entry point for the library.
///
/// Should not be invoked directly, but via the `hook!` macro which will
/// also provide the `WinMain` entry point.
///
/// This function finds the `IDXGISwapChain::Present` function address,
/// creates and enables the hook via `MinHook`. Returns the callback to the
/// trampoline function, if successful.
pub unsafe fn apply_hook(render_loop: Box<dyn RenderLoop>) {
  if RENDER_LOOP.set(Mutex::new(render_loop)).is_err() {
    panic!("Render loop already assigned");
  }

  let xinput_addr = get_xinput_addr();
  info!("XInputGetState = {:p}", xinput_addr);

  let dxgi_swap_chain_present_addr = get_present_addr();
  info!(
    "IDXGISwapChain::Present = {:p}",
    dxgi_swap_chain_present_addr
  );

  let mut xinput_get_state_trampoline = null_mut();
  let mut dxgi_swap_chain_present_trampoline = null_mut();

  let status = mh::MH_Initialize();
  info!("MH_Initialize: {:?}", status);

  // Hook IDXGISwapCHain::Present
  let status = mh::MH_CreateHook(
    dxgi_swap_chain_present_addr,
    dxgi_swap_chain_present_impl as LPVOID,
    &mut dxgi_swap_chain_present_trampoline as *mut _ as _,
  );
  info!("MH_CreateHook: {:?}", status);
  let status = mh::MH_QueueEnableHook(dxgi_swap_chain_present_addr);
  info!("MH_QueueEnableHook: {:?}", status);

  // Hook XInputGetState
  let status = mh::MH_CreateHook(
    xinput_addr,
    xinput_get_state_impl as LPVOID,
    &mut xinput_get_state_trampoline as *mut _ as _,
  );
  info!("MH_CreateHook: {:?}", status);
  let status = mh::MH_QueueEnableHook(xinput_addr);
  info!("MH_QueueEnableHook: {:?}", status);

  let status = mh::MH_ApplyQueued();
  info!("MH_ApplyQueued: {:?}", status);

  if DXGI_SWAP_CHAIN_TRAMPOLINE
    .set(std::mem::transmute::<_, DXGISwapChainPresentType>(
      dxgi_swap_chain_present_trampoline,
    ))
    .is_err()
  {
    panic!("IDXGISwapChain::Present trampoline already assigned");
  }

  if XINPUT_GET_STATE_TRAMPOLINE
    .set(std::mem::transmute::<*mut c_void, XInputGetStateType>(
      xinput_get_state_trampoline,
    ))
    .is_err()
  {
    panic!("XInputGetState trampoline already assigned");
  }
}
