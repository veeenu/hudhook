use once_cell::sync::OnceCell;
use std::ptr::null;
use detour::RawDetour;
use imgui::Context;
use log::{error};
use parking_lot::Mutex;
use windows::core::{HRESULT, Interface, PCSTR};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct3D9::{D3D_SDK_VERSION, D3DADAPTER_DEFAULT, D3DCREATE_SOFTWARE_VERTEXPROCESSING, D3DDEVTYPE_HAL, D3DDISPLAYMODE, D3DFORMAT, D3DPRESENT_PARAMETERS, D3DSWAPEFFECT_DISCARD, Direct3DCreate9, IDirect3DDevice9};
use windows::Win32::Graphics::Gdi::{HBRUSH, RGNDATA};
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExA, CS_HREDRAW, CS_OWNDC, CS_VREDRAW, DefWindowProcA, DestroyWindow, GetCursorPos, GetForegroundWindow, HCURSOR, HICON, HMENU, RegisterClassA, WINDOW_EX_STYLE, WNDCLASSA, WS_OVERLAPPEDWINDOW, WS_VISIBLE};
use crate::hooks::common::{ImguiWindowsEventHandler, WndProcType};
use crate::hooks::{Hooks, ImguiRenderLoop, ImguiRenderLoopFlags};



type Dx9EndSceneFn = unsafe extern "system" fn(this: IDirect3DDevice9,) -> HRESULT;

type Dx9PresentFn = unsafe extern "system" fn(
    this: IDirect3DDevice9,
    psourcerect: *const RECT,
    pdestrect: *const RECT,
    hdestwindowoverride: HWND,
    pdirtyregion: *const RGNDATA
) -> HRESULT;

unsafe extern "system" fn imgui_dx9_end_scene_impl(this: IDirect3DDevice9) -> HRESULT
{
    unsafe extern "system" fn def_window_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        DefWindowProcA(hwnd, msg, wparam, lparam)
    }

    let mut imgui_renderer = IMGUI_RENDERER.get_or_init(|| {
        let mut context = imgui::Context::create();
        context.set_ini_filename(None);
        let renderer = imgui_dx9::Renderer::new(&mut context, this.clone()).unwrap();

        Mutex::new(Box::new(
            ImguiRenderer
            {
                ctx: context,
                renderer,
                wnd_proc: def_window_proc,
                flags: ImguiRenderLoopFlags { focused: false }
            }
        ))
    }).lock();

   imgui_renderer.render();



        //Ok(imgui_renderer) =>
        //{
        //    let r = imgui_renderer.unwrap();
        //    r.renderer.render(r.ctx);
        //}
        //_ => error!("Failed to acquire imgui_renderer lock")





    let (trampoline_end_scene, _) = TRAMPOLINE.get().expect("dx9_Present trampoline uninitialized");
    let result = trampoline_end_scene(this);
    return result;
}


unsafe extern "system" fn imgui_dx9_present_impl(
    this: IDirect3DDevice9,
    psourcerect: *const RECT,
    pdestrect: *const RECT,
    hdestwindowoverride: HWND,
    pdirtyregion: *const RGNDATA
) -> HRESULT
{
    let (_, trampoline_present) = TRAMPOLINE.get().expect("dx9_Present trampoline uninitialized");
    let result = trampoline_present(this, psourcerect, pdestrect, hdestwindowoverride, pdirtyregion);
    return result;
}

static mut IMGUI_RENDER_LOOP: OnceCell<Box<dyn ImguiRenderLoop + Send + Sync>> = OnceCell::new();
static mut IMGUI_RENDERER: OnceCell<Mutex<Box<ImguiRenderer>>> = OnceCell::new();
static TRAMPOLINE: OnceCell<(Dx9EndSceneFn, Dx9PresentFn)> = OnceCell::new();



struct ImguiRenderer
{
    ctx: Context,
    renderer: imgui_dx9::Renderer,
    wnd_proc: WndProcType,
    flags: ImguiRenderLoopFlags,
}

impl ImguiRenderer
{
    unsafe fn render(&mut self)
    {


        {
            let mut io = self.ctx.io_mut();
            let rect = self.renderer.get_window_rect();
            io.display_size = [(rect.right - rect.left) as f32, (rect.bottom - rect.top) as f32];
        }


        let mut pos = POINT { x: 0, y: 0 };

        //let active_window = GetForegroundWindow();

        //let gcp = GetCursorPos(&mut pos as *mut _);
        //if gcp.as_bool() && ScreenToClient(sd.OutputWindow, &mut pos as *mut _).as_bool() {
        //    io.mouse_pos[0] = pos.x as _;
        //    io.mouse_pos[1] = pos.y as _;
        //}


        //let ctx = &mut self.ctx;
        let mut ui = self.ctx.frame();

        IMGUI_RENDER_LOOP.get_mut().unwrap().render(&mut ui, &self.flags);
        let draw_data = ui.render();
        self.renderer.render(draw_data).unwrap();
    }

    unsafe fn cleanup(&mut self){}
}

impl ImguiWindowsEventHandler for ImguiRenderer
{
    fn io(&self) -> &imgui::Io {
        self.ctx.io()
    }

    fn io_mut(&mut self) -> &mut imgui::Io {
        self.ctx.io_mut()
    }

    fn focus(&self) -> bool {
        self.flags.focused
    }

    fn focus_mut(&mut self) -> &mut bool {
        &mut self.flags.focused
    }

    fn wnd_proc(&self) -> WndProcType {
        self.wnd_proc
    }
}
unsafe impl Send for ImguiRenderer {}
unsafe impl Sync for ImguiRenderer {}


/// Stores hook detours and implements the [`Hooks`] trait.
pub struct ImguiDX9Hooks
{
    hook_dx9_end_scene: RawDetour,
    hook_dx9_present: RawDetour,
}

impl ImguiDX9Hooks
{
    pub unsafe fn new<T: 'static>(t: T) -> Self
        where T: ImguiRenderLoop + Send + Sync,
    {
        let (hook_dx9_end_scene_address, dx9_present_address)  = get_dx9_present_addr();

        let  hook_dx9_end_scene = RawDetour::new(
            hook_dx9_end_scene_address as *const _,
            imgui_dx9_end_scene_impl as *const _,
        ) .expect("dx9_end_scene hook");

        let  hook_dx9_present = RawDetour::new(
            dx9_present_address as *const _,
            imgui_dx9_present_impl as *const _,
        ) .expect("dx9_present hook");

        IMGUI_RENDER_LOOP.get_or_init(|| Box::new(t));
        TRAMPOLINE.get_or_init(|| {(
                std::mem::transmute(hook_dx9_end_scene.trampoline()),
                std::mem::transmute(hook_dx9_present.trampoline()),

        )});

        Self {  hook_dx9_end_scene, hook_dx9_present }
    }
}


impl Hooks for ImguiDX9Hooks
{
    unsafe fn hook(&self)
    {
        for hook in [&self.hook_dx9_end_scene, &self.hook_dx9_present] {
            if let Err(e) = hook.enable() {
                error!("Couldn't enable hook: {e}");
            }
        }
    }

    unsafe fn unhook(&mut self)
    {
        for hook in [&self.hook_dx9_end_scene, &self.hook_dx9_present] {
            if let Err(e) = hook.disable() {
                error!("Couldn't disable hook: {e}");
            }
        }

        if let Some(renderer) = IMGUI_RENDERER.take() {
            renderer.lock().cleanup();
        }
        drop(IMGUI_RENDER_LOOP.take());
    }
}


////////////////////////////////////////////////////////////////////////////////////////////////////
// Function address finders
////////////////////////////////////////////////////////////////////////////////////////////////////

unsafe fn create_dummy_window() -> HWND
{
    unsafe extern "system" fn def_window_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        DefWindowProcA(hwnd, msg, wparam, lparam)
    }

    let hwnd = {
        let hinstance =  GetModuleHandleA(None);
        let wnd_class = WNDCLASSA {
            style: CS_OWNDC | CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(def_window_proc),
            hInstance: hinstance,
            lpszClassName: PCSTR("HUDHOOK_DUMMY\0".as_ptr()),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hIcon: HICON(0),
            hCursor: HCURSOR(0),
            hbrBackground: HBRUSH(0),
            lpszMenuName: PCSTR(null()),
        };

        RegisterClassA(&wnd_class);
        CreateWindowExA(
            WINDOW_EX_STYLE(0),
            PCSTR("HUDHOOK_DUMMY\0".as_ptr()),
            PCSTR("HUDHOOK_DUMMY\0".as_ptr()),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            0,
            0,
            100,
            100,
            HWND(0),
            HMENU(0),
            hinstance,
            null(),
        )
    };
    return hwnd;
}

unsafe fn get_dx9_present_addr() -> (Dx9EndSceneFn, Dx9PresentFn)
{
    let hwnd = create_dummy_window();

    let d9 = Direct3DCreate9(D3D_SDK_VERSION).unwrap();

    let mut d3d_display_mode = D3DDISPLAYMODE{
        Width: 0,
        Height: 0,
        RefreshRate: 0,
        Format: D3DFORMAT(0),
    };
    d9.GetAdapterDisplayMode(D3DADAPTER_DEFAULT, &mut d3d_display_mode).unwrap();

    let mut present_params =  D3DPRESENT_PARAMETERS
    {
        Windowed: BOOL(1),
        SwapEffect: D3DSWAPEFFECT_DISCARD,
        BackBufferFormat: d3d_display_mode.Format,
        ..core::mem::zeroed()
    };

    let mut device: Option<IDirect3DDevice9> = None;
    d9.CreateDevice(
        D3DADAPTER_DEFAULT,
        D3DDEVTYPE_HAL,
        hwnd,
        D3DCREATE_SOFTWARE_VERTEXPROCESSING as u32,
        &mut present_params,
        &mut device,
    ).expect("dx9 failed to create device");
    let device = device.unwrap();

    let end_scene_ptr = device.vtable().EndScene;
    let present_ptr = device.vtable().Present;

    DestroyWindow(hwnd);
    return (std::mem::transmute(end_scene_ptr), std::mem::transmute(present_ptr));
}