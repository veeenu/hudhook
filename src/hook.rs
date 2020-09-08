use log::*;

use winapi::shared::dxgi::*;
use winapi::shared::dxgiformat::*;
use winapi::shared::dxgitype::*;
use winapi::shared::minwindef::*;
use winapi::shared::windef::{HWND, RECT};
use winapi::um::d3d11::*;
use winapi::um::d3dcommon::*;
use winapi::um::winnt::*;
use winapi::um::winuser::*;
use winapi::Interface;

use core::mem::MaybeUninit;

use std::cell::Cell;
use std::ptr::null_mut;

use crate::imgui_impl;
use crate::mh;
use crate::util::Error;

type Result<T> = std::result::Result<T, Error>;

type IDXGISwapChainPresent =
  unsafe extern "system" fn(This: *mut IDXGISwapChain, SyncInterval: UINT, Flags: UINT) -> HRESULT;
type WndProc =
  unsafe extern "system" fn(hwnd: HWND, umsg: UINT, wparam: WPARAM, lparam: LPARAM) -> isize;

/// Data structure to hold all info we need at frame render time.

pub(crate) struct DxgiHook {
  present_trampoline: IDXGISwapChainPresent,
  default_wnd_proc: WndProc,
  p_device_context: *mut ID3D11DeviceContext,
  render_target_view: *mut ID3D11RenderTargetView,
  imgui_ctx: imgui::Context,
  renderer: imgui_impl::dx11::Renderer,
  render_loop: Box<dyn RenderLoop>,
  // p_device: *mut ID3D11Device,
}

/// State machine for the initialization status of the DXGI hook.

enum DxgiHookState {
  Uninitialized,
  Hooked(IDXGISwapChainPresent, Box<dyn RenderLoop>),
  Errored(IDXGISwapChainPresent),
  Ok(DxgiHook),
}

impl Default for DxgiHookState {
  fn default() -> DxgiHookState {
    DxgiHookState::Uninitialized
  }
}

// why does it have to be static FeelsBadMan
static mut DXGI_HOOK_STATE: Cell<DxgiHookState> = Cell::new(DxgiHookState::Uninitialized);

impl DxgiHook {
  /// Initialize the DXGI hook.

  // TODO URGENT if Result is Err, caller must call present_trampoline
  fn initialize_dx(
    present_trampoline: IDXGISwapChainPresent,
    p_this: *mut IDXGISwapChain,
    render_loop: Box<dyn RenderLoop>,
  ) -> Result<DxgiHook> {
    debug!("Initializing DXGI hook");
    let this =
      unsafe { p_this.as_ref() }.ok_or_else(|| Error(format!("Null IDXGISwapChain reference")))?;
    let mut ui: UINT = 0;
    unsafe { this.GetLastPresentCount(&mut ui) };

    let mut p_device = null_mut();
    let mut p_device_context = null_mut();
    let dev = unsafe { this.GetDevice(&ID3D11Device::uuidof(), &mut p_device) };
    if dev < 0 {
      return Err(Error(format!(
        "Get device + ctx from swap chain failed: {:?} {:?}",
        dev, p_this
      )));
    };

    let p_device = p_device as *mut ID3D11Device;
    unsafe { (*p_device).GetImmediateContext(&mut p_device_context) };

    let p_device_context = p_device_context as *mut ID3D11DeviceContext;

    let mut sd: DXGI_SWAP_CHAIN_DESC = unsafe { std::mem::zeroed() };
    unsafe { this.GetDesc(&mut sd as _) };

    let default_wnd_proc = unsafe {
      std::mem::transmute(SetWindowLongPtrA(
        sd.OutputWindow,
        GWLP_WNDPROC,
        wnd_proc as WndProc as isize,
      ))
    };

    let mut imgui_ctx = imgui::Context::create();
    imgui_ctx.set_ini_filename(None);
    imgui_ctx
      .fonts()
      .add_font(&[imgui::FontSource::DefaultFontData {
        config: Some(imgui::FontConfig {
          ..imgui::FontConfig::default()
        }),
      }]);

    let renderer = imgui_impl::dx11::Renderer::new(p_device, p_device_context, &mut imgui_ctx)?;

    let mut back_buf: *mut ID3D11Texture2D = null_mut();
    let mut render_target_view: *mut ID3D11RenderTargetView = null_mut();
    unsafe {
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
    }

    debug!("Initialization completed");
    Ok(DxgiHook {
      present_trampoline,
      default_wnd_proc,
      // p_device,
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

  fn render(&mut self, this: *mut IDXGISwapChain, sync_interval: UINT, flags: UINT) -> HRESULT {
    // SAFETY
    // idk lmao
    unsafe {
      self.p_device_context.as_ref().unwrap().OMSetRenderTargets(
        1,
        &mut self.render_target_view as *mut *mut _,
        null_mut(),
      );
    }
    // SAFETY
    // No reason this.as_ref() should error at this point, and probably it's a
    // good idea to crash and burn if it does. TODO check
    let mut sd: DXGI_SWAP_CHAIN_DESC = unsafe { std::mem::zeroed() };
    unsafe { this.as_ref().unwrap().GetDesc(&mut sd as _) };

    let mut rect: RECT = unsafe { std::mem::zeroed() };
    unsafe { GetWindowRect(sd.OutputWindow, &mut rect as _) };

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

    debug!("Calling render loop");
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

    debug!("Rendering frame data");
    let dd = ui.render();

    debug!("Displaying image data");
    match self.renderer.render(dd) {
      Ok(_) => {}
      Err(e) => error!("Renderer errored: {:?}", e),
    };

    unsafe { (self.present_trampoline)(this, sync_interval, flags) }
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

/// Implementation of the hooked `Present` function.
///
/// Implements a state machine to move the hook from uninitialized, to
/// hooked, to rendering or errored.

extern "system" fn present_impl(
  this: *mut IDXGISwapChain,
  sync_interval: UINT,
  flags: UINT,
) -> HRESULT {
  // State transition the dxgi hook struct
  unsafe {
    DXGI_HOOK_STATE.replace(match DXGI_HOOK_STATE.take() {
      DxgiHookState::Uninitialized => {
        unreachable!("DXGI Hook State uninitialized in present_impl -- this should never happen!")
      }
      DxgiHookState::Hooked(present_trampoline, render_loop) => {
        // Stuff like this should be put in a callback and passed to hook().
        /*
        // Base: 7ff7762a0000
        // (Base + 0x0001BAF0, 0x10) float a
        // (Base + 0x0001BAF0, 0x18) double b
        // (Base + 0x0001BAF0, 0x20) uint64 c
        let pc_u32 = PointerChain::<u32>::new(vec![
          // 0x7ff7762a0000 + 0x1BAF0,
          get_base_address::<u8>() as usize as isize + 0x1baf0,
          0x20
        ]);
        debug!("Read value {:?}", pc_u32.read());
        pc_u32.write(31337);
        debug!("Read value {:?} after writing", pc_u32.read());
        */

        match DxgiHook::initialize_dx(present_trampoline, this, render_loop) {
          Ok(dh) => DxgiHookState::Ok(dh),
          Err(e) => {
            error!("DXGI Hook initialization failed: {:?}", e);
            DxgiHookState::Errored(present_trampoline)
          }
        }
      }
      DxgiHookState::Errored(present_trampoline) => DxgiHookState::Errored(present_trampoline),
      DxgiHookState::Ok(dh) => DxgiHookState::Ok(dh),
    })
  };

  match unsafe { DXGI_HOOK_STATE.get_mut() } {
    DxgiHookState::Uninitialized => unreachable!(),
    DxgiHookState::Hooked(_, _) => unreachable!(),
    DxgiHookState::Errored(present_trampoline) => unsafe {
      present_trampoline(this, sync_interval, flags)
    },
    DxgiHookState::Ok(dxgi_hook) => dxgi_hook.render(this, sync_interval, flags),
  }
}

/// Get the `IDXGISwapChain::Present` function address.
///
/// Creates a swap chain + device instance and looks up its
/// vtable to find the address.

fn get_present_address() -> Result<IDXGISwapChainPresent> {
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
  swap_chain_desc.OutputWindow = unsafe { GetForegroundWindow() };
  swap_chain_desc.SampleDesc.Count = 1;
  swap_chain_desc.Windowed = 1;

  let result = unsafe {
    D3D11CreateDeviceAndSwapChain(
      0 as *mut IDXGIAdapter,
      D3D_DRIVER_TYPE_HARDWARE,
      0 as HMODULE,
      0 as UINT,
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
      "D3D11CreateDeviceAndSwapChain failed {:?}",
      result
    )));
  }

  let ret = unsafe { (*(&*p_swap_chain).lpVtbl).Present };
  unsafe {
    (*p_device).Release();
    (*p_context).Release();
    (*p_swap_chain).Release();
  }

  Ok(ret)
}

// ==================
// === PUBLIC API ===
// ==================

/// Interface for implementing the "game loop".
///
/// The `render` method of this trait will be invoked once per frame. Memory
/// management and UI visualization (via the current frame's `imgui::Ui`
/// instance) should be made inside of it.

pub trait RenderLoop {
  fn render<'a>(&mut self, ctx: RenderContext);
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

pub fn apply_hook(render_loop: Box<dyn RenderLoop>) -> Result<IDXGISwapChainPresent> {
  debug!("Starting hook");
  let present_original = get_present_address()?;
  let mut present_trampoline: MaybeUninit<IDXGISwapChainPresent> = MaybeUninit::uninit();
  debug!("Initializing MH");
  let mut status: mh::MH_STATUS = unsafe { mh::MH_Initialize() };
  debug!("MH_Initialize status: {:?}", status);
  status = unsafe {
    mh::MH_CreateHook(
      present_original as LPVOID,
      present_impl as LPVOID,
      &mut present_trampoline as *mut _ as _,
    )
  };
  let present_trampoline = unsafe { present_trampoline.assume_init() };
  unsafe {
    DXGI_HOOK_STATE.replace(DxgiHookState::Hooked(present_trampoline, render_loop));
  }
  debug!("MH_CreateHook status: {:?}", status);
  status = unsafe { mh::MH_EnableHook(present_original as LPVOID) };
  debug!("MH_EnableHook status: {:?}", status);

  Ok(present_trampoline)
}
