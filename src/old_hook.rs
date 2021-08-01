use log::*;

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
use winapi::Interface;
use winapi::um::xinput::XINPUT_STATE;
use winapi::um::xinput::XInputGetState;

use core::mem::MaybeUninit;

use std::cell::Cell;
use std::ffi::c_void;
use std::ffi::CString;
use std::ptr::null_mut;

use crate::imgui_impl;
use crate::mh;
use crate::util::Error;

type Result<T> = std::result::Result<T, Error>;

type XInputGetStateType =
  unsafe extern "system" fn(dw_user_index: DWORD, p_state: *mut XINPUT_STATE) -> DWORD;

type DXGISwapChainPresentType =
  unsafe extern "system" fn(This: *mut IDXGISwapChain, SyncInterval: UINT, Flags: UINT) -> HRESULT;

// type IDXGISwapChainPresent =
//   unsafe extern "system" fn(This: *mut IDXGISwapChain, SyncInterval: UINT, Flags: UINT) -> HRESULT;
type WndProc =
  unsafe extern "system" fn(hwnd: HWND, umsg: UINT, wparam: WPARAM, lparam: LPARAM) -> isize;

/// Data structure to hold all info we need at frame render time.
pub(crate) struct DxgiHook {
  present_trampoline: DXGISwapChainPresentType,
  default_wnd_proc: WndProc,
  p_device_context: *mut ID3D11DeviceContext,
  render_target_view: *mut ID3D11RenderTargetView,
  imgui_ctx: imgui::Context,
  renderer: imgui_impl::dx11::Renderer,
  render_loop: Box<dyn RenderLoop>,
}

/// State machine for the initialization status of the DXGI hook.
enum DxgiHookState {
  Uninitialized,
  Hooked(
    DXGISwapChainPresentType,
    XInputGetStateType,
    Box<dyn RenderLoop>,
  ),
  Errored(DXGISwapChainPresentType),
  Ok(Box<DxgiHook>),
}

impl Default for DxgiHookState {
  fn default() -> DxgiHookState {
    DxgiHookState::Uninitialized
  }
}

// why does it have to be static FeelsBadMan
static mut DXGI_HOOK_STATE: Cell<DxgiHookState> = Cell::new(DxgiHookState::Uninitialized);

fn cast_dptr<T>(t: &mut *mut T) -> *mut *mut c_void {
  t as *mut *mut T as *mut *mut c_void
}

impl DxgiHook {
  /// Initialize the DXGI hook.
  // TODO URGENT if Result is Err, caller must call present_trampoline
  unsafe fn initialize_dx(
    present_trampoline: DXGISwapChainPresentType,
    p_this: *mut IDXGISwapChain,
    render_loop: Box<dyn RenderLoop>,
  ) -> Result<DxgiHook> {
    trace!("Initializing DXGI hook");

    let this = &*p_this;
    let mut p_device: *mut ID3D11Device = null_mut();
    let mut p_device_context: *mut ID3D11DeviceContext = null_mut();
    let mut sd: DXGI_SWAP_CHAIN_DESC = std::mem::zeroed();
    let mut back_buf: *mut ID3D11Texture2D = null_mut();
    let mut render_target_view: *mut ID3D11RenderTargetView = null_mut();

    let mut lpc: UINT = 0;
    this.GetLastPresentCount(&mut lpc);

    let result = this.GetDevice(&ID3D11Device::uuidof(), cast_dptr(&mut p_device));
    if result < 0 {
      return Err(Error(format!(
        "Get device + ctx from swap chain failed: {:?} {:?}",
        result, p_this
      )));
    };

    (*p_device).GetImmediateContext(&mut p_device_context);

    this.GetDesc(&mut sd as _);

    #[allow(clippy::fn_to_numeric_cast)]
    let default_wnd_proc = std::mem::transmute(SetWindowLongPtrA(
      sd.OutputWindow,
      GWLP_WNDPROC,
      wnd_proc as _,
    ));

    let mut imgui_ctx = imgui::Context::create();
    imgui_ctx.set_ini_filename(None);
    imgui_ctx
      .fonts()
      .add_font(&[imgui::FontSource::DefaultFontData {
        config: Some(imgui::FontConfig {
          ..imgui::FontConfig::default()
        }),
      }]);
    imgui_ctx
      .fonts()
      .add_font(&[imgui::FontSource::DefaultFontData {
        config: Some(imgui::FontConfig {
          size_pixels: 26.,
          ..imgui::FontConfig::default()
        }),
      }]);

    let renderer = imgui_impl::dx11::Renderer::new(p_device, p_device_context, &mut imgui_ctx)?;

    this.GetBuffer(
      0,
      &ID3D11Texture2D::uuidof(),
      &mut back_buf as *mut *mut _ as _,
    );
    (*p_device).CreateRenderTargetView(
      back_buf as _,
      null_mut(),
      &mut render_target_view as *mut *mut _ as _,
    );
    (*back_buf).Release();

    trace!("Initialization completed");
    Ok(DxgiHook {
      present_trampoline,
      default_wnd_proc,
      p_device_context,
      render_target_view,
      imgui_ctx,
      renderer,
      render_loop,
    })
  }

  /// Render loop function.
  ///
  /// This function is called in place of the regular `IDXGISwapChain::Present`
  /// function and is responsible for finally calling the trampoline and
  /// letting the game run its own code.
  unsafe fn render(
    &mut self,
    p_this: *mut IDXGISwapChain,
    sync_interval: UINT,
    flags: UINT,
  ) -> HRESULT {
    let this = &*p_this;
    let mut sd: DXGI_SWAP_CHAIN_DESC = std::mem::zeroed();
    let mut rect: RECT = std::mem::zeroed();

    // SAFETY
    // idk lmao
    (*self.p_device_context).OMSetRenderTargets(
      1,
      &mut self.render_target_view as *mut *mut _,
      null_mut(),
    );
    // SAFETY
    // No reason this.as_ref() should error at this point, and probably it's a
    // good idea to crash and burn if it does. TODO check
    this.GetDesc(&mut sd as _);

    if GetWindowRect(sd.OutputWindow, &mut rect as _) != 0 {
      let mut io = self.imgui_ctx.io_mut();

      io.display_size = [
        (rect.right - rect.left) as f32,
        (rect.bottom - rect.top) as f32,
      ];

      let io = self.imgui_ctx.io();
      let keys_down = io
        .keys_down
        .iter()
        .enumerate()
        .filter_map(|(idx, &val)| if val { Some(idx) } else { None })
        .collect::<Vec<_>>();
      let imgui::Io {
        key_ctrl,
        key_shift,
        key_alt,
        key_super,
        display_size,
        ..
      } = *io;

      trace!("Calling render loop");
      set_mouse_pos(&mut self.imgui_ctx, sd.OutputWindow);
      let ui = self.imgui_ctx.frame();

      self.render_loop.render(RenderContext {
        frame: &ui,
        key_ctrl,
        key_shift,
        key_alt,
        key_super,
        keys_down,
        display_size,
      });

      trace!("Rendering frame data");
      let dd = ui.render();

      if self.render_loop.is_visible() {
        trace!("Displaying image data");
        match self.renderer.render(dd) {
          Ok(_) => {}
          Err(e) => error!("Renderer errored: {:?}", e),
        };
      }
    }

    (self.present_trampoline)(p_this, sync_interval, flags)
  }
}

fn set_mouse_pos(ctx: &mut imgui::Context, hwnd: HWND) {
  let io = ctx.io_mut();
  let mut pos = POINT { x: 0, y: 0 };

  let active_window = unsafe { GetForegroundWindow() };
  if active_window != 0 as HWND
    && (active_window == hwnd || unsafe { IsChild(active_window, hwnd) != 0 })
  {
    let gcp = unsafe { GetCursorPos(&mut pos as *mut _) };
    if gcp != 0 && unsafe { ScreenToClient(hwnd, &mut pos as *mut _) } != 0 {
      io.mouse_pos[0] = pos.x as _;
      io.mouse_pos[1] = pos.y as _;
    }
  }
}

/// Placeholder `WndProc`.
///
/// Currently processes keydown and keyup events.
unsafe extern "system" fn wnd_proc(
  hwnd: HWND,
  umsg: UINT,
  wparam: WPARAM,
  lparam: LPARAM,
) -> isize {
  if let DxgiHookState::Ok(hook) = DXGI_HOOK_STATE.get_mut() {
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
        set_capture(&hook.imgui_ctx.io().mouse_down, hwnd);
        hook.imgui_ctx.io_mut().mouse_down[0] = true;
        return 1;
      }
      WM_RBUTTONDOWN | WM_RBUTTONDBLCLK => {
        set_capture(&hook.imgui_ctx.io().mouse_down, hwnd);
        hook.imgui_ctx.io_mut().mouse_down[1] = true;
        return 1;
      }
      WM_MBUTTONDOWN | WM_MBUTTONDBLCLK => {
        set_capture(&hook.imgui_ctx.io().mouse_down, hwnd);
        hook.imgui_ctx.io_mut().mouse_down[2] = true;
        return 1;
      }
      WM_XBUTTONDOWN | WM_XBUTTONDBLCLK => {
        let btn = if GET_XBUTTON_WPARAM(wparam) == XBUTTON1 {
          3
        } else {
          4
        };
        set_capture(&hook.imgui_ctx.io().mouse_down, hwnd);
        hook.imgui_ctx.io_mut().mouse_down[btn] = true;
        return 1;
      }
      WM_LBUTTONUP => {
        hook.imgui_ctx.io_mut().mouse_down[0] = false;
        release_capture(&hook.imgui_ctx.io().mouse_down, hwnd);
        return 1;
      }
      WM_RBUTTONUP => {
        hook.imgui_ctx.io_mut().mouse_down[1] = false;
        release_capture(&hook.imgui_ctx.io().mouse_down, hwnd);
        return 1;
      }
      WM_MBUTTONUP => {
        hook.imgui_ctx.io_mut().mouse_down[2] = false;
        release_capture(&hook.imgui_ctx.io().mouse_down, hwnd);
        return 1;
      }
      WM_XBUTTONUP => {
        let btn = if GET_XBUTTON_WPARAM(wparam) == XBUTTON1 {
          3
        } else {
          4
        };
        hook.imgui_ctx.io_mut().mouse_down[btn] = false;
        release_capture(&hook.imgui_ctx.io().mouse_down, hwnd);
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

    CallWindowProcW(Some(hook.default_wnd_proc), hwnd, umsg, wparam, lparam)
  } else {
    0
  }
}

#[allow(non_snake_case)]
extern "system" fn XInputGetStateOverride(
  dw_user_index: DWORD,
  p_state: *mut XINPUT_STATE,
) -> DWORD {
  let mut state: XINPUT_STATE = unsafe { std::mem::zeroed() };
  let retval = unsafe { XInputGetState(dw_user_index, &mut state as *mut _) };

  if let Some(m) = unsafe { p_state.as_mut() } {
    *m = state;
  }

  retval
}

/// Implementation of the hooked `Present` function.
///
/// Implements a state machine to move the hook from uninitialized, to
/// hooked, to rendering or errored.
#[allow(non_snake_case)]
unsafe extern "system" fn DXGISwapChainPresentOverride(
  this: *mut IDXGISwapChain,
  sync_interval: UINT,
  flags: UINT,
) -> HRESULT {
  // State transition the dxgi hook struct
  DXGI_HOOK_STATE.replace(match DXGI_HOOK_STATE.take() {
    DxgiHookState::Uninitialized => {
      unreachable!("DXGI Hook State uninitialized in present_impl -- this should never happen!")
    }
    DxgiHookState::Hooked(present_trampoline, xigs_trampoline, render_loop) => {
      match DxgiHook::initialize_dx(present_trampoline, this, render_loop) {
        Ok(dh) => DxgiHookState::Ok(Box::new(dh)),
        Err(e) => {
          error!("DXGI Hook initialization failed: {:?}", e);
          DxgiHookState::Errored(present_trampoline)
        }
      }
    }
    DxgiHookState::Errored(present_trampoline) => DxgiHookState::Errored(present_trampoline),
    DxgiHookState::Ok(dh) => DxgiHookState::Ok(dh),
  });

  match DXGI_HOOK_STATE.get_mut() {
    DxgiHookState::Uninitialized => unreachable!(),
    DxgiHookState::Hooked(_, _, _) => unreachable!(),
    DxgiHookState::Errored(present_trampoline) => present_trampoline(this, sync_interval, flags),
    DxgiHookState::Ok(dxgi_hook) => dxgi_hook.render(this, sync_interval, flags),
  }
}

/// Get the `IDXGISwapChain::Present` function address.
///
/// Creates a swap chain + device instance and looks up its
/// vtable to find the address.
fn get_present_addr() -> Result<LPVOID> {
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
    return Err(Error(format!(
      "D3D11CreateDeviceAndSwapChain failed {:x}",
      result
    )));
  }

  let ret = unsafe { (*(*p_swap_chain).lpVtbl).Present };

  unsafe {
    (*p_device).Release();
    (*p_context).Release();
    (*p_swap_chain).Release();
    DestroyWindow(hwnd);
  }

  Ok(ret as LPVOID)
}

unsafe fn get_xinput_addr() -> LPVOID {
  let xinput_dll = LoadLibraryA(CString::new("xinput1_3.dll").unwrap().as_c_str().as_ptr());
  let xinput_addr = GetProcAddress(
    xinput_dll,
    CString::new("XInputGetState").unwrap().as_c_str().as_ptr(),
  );
  xinput_addr as _
}

// ==================
// === PUBLIC API ===
// ==================

/// Interface for implementing the render loop.
///

pub trait RenderLoop {
  /// Invoked once per frame. Memory management and UI visualization (via the
  /// current frame's `imgui::Ui` instance) should be made inside of it.
  fn render(&mut self, ctx: RenderContext);

  /// Return `true` when you want your UI to be rendered to screen.
  ///
  /// The [`render`](#tyrender) method will still be called, but the draw data
  /// will not be displayed
  fn is_visible(&self) -> bool;

  /// Return `true` when you want the underlying application to stop receiving
  /// `WndProc` events. Presently not functioning.
  fn is_capturing(&self) -> bool;
}

/// Information context made available to the RenderLoop
///
/// For now, it is a subset of the `imgui` context crafted in such a way that
/// it is difficult to break the (limited) intended way of operating the library.

pub struct RenderContext<'a> {
  pub frame: &'a imgui::Ui<'a>,
  pub key_ctrl: bool,
  pub key_shift: bool,
  pub key_alt: bool,
  pub key_super: bool,
  pub keys_down: Vec<usize>,
  pub display_size: [f32; 2],
}

/// Inner entry point for the library.
///
/// Should not be invoked directly, but via the `hook!` macro which will
/// also provide the `WinMain` entry point.
///
/// This function finds the `IDXGISwapChain::Present` function address,
/// creates and enables the hook via `MinHook`. Returns the callback to the
/// trampoline function, if successful.

pub unsafe fn apply_hook(
  render_loop: Box<dyn RenderLoop>,
) -> Result<(DXGISwapChainPresentType, XInputGetStateType)> {

  let xinput_addr = get_xinput_addr();
  info!("XInputGetState = {:p}", xinput_addr);

  let dxgi_swap_chain_present_addr = get_present_addr().unwrap();
  info!(
    "IDXGISwapChain::Present = {:p}",
    dxgi_swap_chain_present_addr
  );

  let mut xinput_get_state_trampoline = MaybeUninit::<XInputGetStateType>::uninit();
  let mut dxgi_swap_chain_present_trampoline = MaybeUninit::<DXGISwapChainPresentType>::uninit();

  let status = mh::MH_Initialize();
  info!("MH_Initialize: {:?}", status);

  // Hook IDXGISwapCHain::Present
  let status = mh::MH_CreateHook(
    dxgi_swap_chain_present_addr,
    DXGISwapChainPresentOverride as LPVOID,
    &mut dxgi_swap_chain_present_trampoline as *mut _ as _,
  );
  info!("MH_CreateHook: {:?}", status);
  let status = mh::MH_QueueEnableHook(dxgi_swap_chain_present_addr);
  info!("MH_QueueEnableHook: {:?}", status);

  // Hook XInputGetState
  let status = mh::MH_CreateHook(
    xinput_addr,
    XInputGetStateOverride as LPVOID,
    &mut xinput_get_state_trampoline as *mut _ as _,
  );
  info!("MH_CreateHook: {:?}", status);
  let status = mh::MH_QueueEnableHook(xinput_addr);
  info!("MH_QueueEnableHook: {:?}", status);

  let status = mh::MH_ApplyQueued();
  info!("MH_ApplyQueued: {:?}", status);

  Ok((
    dxgi_swap_chain_present_trampoline.assume_init(),
    xinput_get_state_trampoline.assume_init(),
  ))
}
