use log::*;

use winapi::um::d3d11::*;
use winapi::um::d3dcommon::*;
use winapi::shared::dxgi::*;
use winapi::shared::dxgiformat::*;
use winapi::shared::dxgitype::*;
use winapi::um::winuser::*;
use winapi::shared::minwindef::*;
use winapi::shared::windef::HWND;
use winapi::um::winnt::*;
use winapi::Interface;

use core::mem::MaybeUninit;

use std::ptr::null_mut;
use std::cell::Cell;

use crate::util::Error;
use crate::imgui_impl;
use crate::mh;

type Result<T> = std::result::Result<T, Error>;

type IDXGISwapChainPresent =
  unsafe extern "system" fn(
    This: *mut IDXGISwapChain, SyncInterval: UINT, Flags: UINT
  ) -> HRESULT;
type WndProc =
  extern "system" fn(hwnd: HWND, umsg: UINT, wparam: WPARAM, lparam: LPARAM);

pub struct DxgiHook {
  present_trampoline: IDXGISwapChainPresent,
  default_wnd_proc: WndProc,
  p_device: *mut ID3D11Device,
  p_device_context: *mut ID3D11DeviceContext,
  render_target_view: *mut ID3D11RenderTargetView,
  imgui_ctx: imgui::Context,
  renderer: imgui_impl::dx11::Renderer
}

enum DxgiHookState {
  Uninitialized,
  Hooked(IDXGISwapChainPresent),
  Errored(IDXGISwapChainPresent),
  Ok(DxgiHook)
}

impl Default for DxgiHookState {
  fn default() -> DxgiHookState {
    DxgiHookState::Uninitialized
  }
}

static mut DXGI_HOOK_STATE: Cell<DxgiHookState> = Cell::new(DxgiHookState::Uninitialized);

impl DxgiHook {

  // TODO URGENT if Result is Err, caller must call present_trampoline
  fn initialize_dx(
    present_trampoline: IDXGISwapChainPresent,
    p_this: *mut IDXGISwapChain
  ) -> Result<DxgiHook> {
    let this = unsafe { p_this.as_ref() }
      .ok_or_else(|| Error(format!("Null IDXGISwapChain reference")))?;
    let mut ui: UINT = 0;
    unsafe { this.GetLastPresentCount(&mut ui) };

    let mut p_device = null_mut();
    let mut p_device_context = null_mut();
    let dev = unsafe { this.GetDevice(&ID3D11Device::uuidof(), &mut p_device) };
    if dev < 0 {
      /*error!("Get device + ctx from swap chain failed: {:?} {:?}", dev, p_this);
      unsafe { (present_trampoline)(p_this, sync_interval, flags) };*/
      return Err(Error(
        format!("Get device + ctx from swap chain failed: {:?} {:?}", dev, p_this)
      ));
    };

    let p_device = p_device as *mut ID3D11Device;
    unsafe { (*p_device).GetImmediateContext(&mut p_device_context) };

    let p_device_context = p_device_context as *mut ID3D11DeviceContext;
    
    let mut sd: DXGI_SWAP_CHAIN_DESC = unsafe { std::mem::zeroed() };
    unsafe { this.GetDesc(&mut sd as _) };

    let default_wnd_proc = unsafe {
      std::mem::transmute(
        SetWindowLongPtrA(sd.OutputWindow, GWLP_WNDPROC, wnd_proc as WndProc as isize)
      )
    };
    /*unsafe {
      SetWindowLongPtrA(
        sd.OutputWindow,
        GWLP_USERDATA,
        default_wnd_proc as WndProc as isize
      );
    }*/

    let mut imgui_ctx = imgui::Context::create();
    imgui_ctx.set_ini_filename(None);
    imgui_ctx.fonts().add_font(&[
      imgui::FontSource::DefaultFontData {
        config: Some(imgui::FontConfig {
          ..imgui::FontConfig::default()
        })
      }
    ]);

    let renderer = imgui_impl::dx11::Renderer::new(
      p_device, p_device_context, &mut imgui_ctx
    )?;

    let mut back_buf: *mut ID3D11Texture2D = null_mut();
    let mut render_target_view: *mut ID3D11RenderTargetView = null_mut();
    unsafe {
      this.GetBuffer(
        0,
        &ID3D11Texture2D::uuidof(),
        &mut back_buf as *mut *mut _ as _
      );
      (*p_device).CreateRenderTargetView(
        back_buf as _, null_mut(), &mut render_target_view as *mut *mut _ as _
      );
      (*back_buf).Release();
    }

    Ok(DxgiHook {
      present_trampoline,
      default_wnd_proc,
      p_device,
      p_device_context,
      render_target_view,
      imgui_ctx,
      renderer
    })
  }

  fn render(
    &mut self,
    this: *mut IDXGISwapChain, sync_interval: UINT, flags: UINT
  ) -> HRESULT {
    unsafe {
      self.p_device_context.as_ref().unwrap().OMSetRenderTargets(
        1, 
        &mut self.render_target_view as *mut *mut _,
        null_mut()
      );
    }
    let mut io = self.imgui_ctx.io_mut();

    io.display_size = [640f32, 480f32];

    let ui = self.imgui_ctx.frame();

    imgui::Window::new(im_str!("Hello"))
      .size([320.0, 256.0], imgui::Condition::FirstUseEver)
      .build(&ui, || {
        ui.text(im_str!("Hello world!"));
        ui.text(im_str!("こんにちは世界！"));
        ui.text(im_str!("This...is...imgui-rs!"));
        ui.separator();
        let mouse_pos = ui.io().mouse_pos;
        ui.text(format!(
          "Mouse Position: ({:.1},{:.1})",
          mouse_pos[0], mouse_pos[1]
        ));
      });

    // ImGui_ImplDX11_RenderDrawData(ui.render() as _);
    let dd = ui.render();
    match self.renderer.render(dd) {
      Ok(_) => {},
      Err(e) => error!("Renderer errored: {:?}", e)
    };

    unsafe { (self.present_trampoline)(this, sync_interval, flags) }
  }
}

extern "system" fn wnd_proc(hwnd: HWND, umsg: UINT, wparam: WPARAM, lparam: LPARAM) {
  /*let default_wndproc: WndProc = unsafe {
    std::mem::transmute(
      GetWindowLongPtrA(hwnd, GWLP_USERDATA)
    )
  };
  default_wndproc(hwnd, umsg, wparam, lparam);*/
  if let DxgiHookState::Ok(hook) = unsafe { DXGI_HOOK_STATE.get_mut() } {
    (hook.default_wnd_proc)(hwnd, umsg, wparam, lparam);
  }
}

extern "system" fn present_impl(
  this: *mut IDXGISwapChain, sync_interval: UINT, flags: UINT
) -> HRESULT {

  // State transition the dxgi hook struct
  unsafe {
    DXGI_HOOK_STATE.replace(match DXGI_HOOK_STATE.take() {
      DxgiHookState::Uninitialized => {
        unreachable!("DXGI Hook State uninitialized in present_impl -- this should never happen!")
      },
      DxgiHookState::Hooked(present_trampoline) => {
        match DxgiHook::initialize_dx(present_trampoline, this) {
          Ok(dh) => DxgiHookState::Ok(dh),
          Err(e) => {
            error!("DXGI Hook initialization failed: {:?}", e);
            DxgiHookState::Errored(present_trampoline)
          }
        }
      },
      DxgiHookState::Errored(present_trampoline) => DxgiHookState::Errored(present_trampoline),
      DxgiHookState::Ok(dh) => DxgiHookState::Ok(dh)
    })
  };

  match unsafe { DXGI_HOOK_STATE.get_mut() } {
    DxgiHookState::Uninitialized => unreachable!(),
    DxgiHookState::Hooked(_) => unreachable!(),
    DxgiHookState::Errored(present_trampoline) => {
      unsafe { present_trampoline(this, sync_interval, flags) }
    },
    DxgiHookState::Ok(dxgi_hook) => {
      dxgi_hook.render(this, sync_interval, flags)
      //unsafe { (dxgi_hook.present_trampoline)(this, sync_interval, flags) }
    }
  }

}

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
      1, D3D11_SDK_VERSION,
      &mut swap_chain_desc as *mut DXGI_SWAP_CHAIN_DESC,
      &mut p_swap_chain as *mut*mut IDXGISwapChain,
      &mut p_device as *mut*mut ID3D11Device,
      null_mut(),
      &mut p_context as *mut*mut ID3D11DeviceContext,
    )
  };

  if result < 0 {
    return Err(Error(
      format!("D3D11CreateDeviceAndSwapChain failed {:?}", result)
    ));
  }

  let ret = unsafe { (*(&*p_swap_chain).lpVtbl).Present };
  unsafe {
    (*p_device).Release();
    (*p_context).Release();
    (*p_swap_chain).Release();
  }

  Ok(ret)
}

// Entry point
pub fn hook() -> Result<IDXGISwapChainPresent> {
  info!("Starting hook");
  let present_original = get_present_address()?;
  let mut present_trampoline: MaybeUninit<IDXGISwapChainPresent> = MaybeUninit::uninit();
  info!("Initializing MH");
  let mut status: mh::MH_STATUS = unsafe { mh::MH_Initialize() };
  info!("MH_Initialize status: {:?}", status);
  status = unsafe {
    mh::MH_CreateHook(
      present_original as LPVOID,
      present_impl as LPVOID,
      &mut present_trampoline as *mut _ as _
    )
  };
  let present_trampoline = unsafe {
    present_trampoline.assume_init()
  };
  unsafe {
    DXGI_HOOK_STATE.replace(DxgiHookState::Hooked(present_trampoline));
  }
  info!("MH_CreateHook status: {:?}", status);
  status = unsafe { mh::MH_EnableHook(present_original as LPVOID) };
  info!("MH_EnableHook status: {:?}", status);

  Ok(present_trampoline)
}