//! General-purpose utilities. These are used across the [`crate`] but have
//! proven useful in client code as well.

use std::ffi::OsString;
use std::fmt::Display;
use std::mem::ManuallyDrop;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use tracing::{debug, error};
use windows::core::s;
use windows::Win32::Foundation::{HANDLE, HMODULE, HWND, MAX_PATH, RECT};
use windows::Win32::Graphics::Direct3D::ID3DBlob;
use windows::Win32::Graphics::Direct3D12::{
    D3D12GetDebugInterface, ID3D12Debug, ID3D12Device, ID3D12Fence, ID3D12Resource,
    D3D12_FENCE_FLAG_NONE, D3D12_RESOURCE_BARRIER, D3D12_RESOURCE_BARRIER_0,
    D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES, D3D12_RESOURCE_BARRIER_FLAG_NONE,
    D3D12_RESOURCE_BARRIER_TYPE_TRANSITION, D3D12_RESOURCE_STATES,
    D3D12_RESOURCE_TRANSITION_BARRIER,
};
use windows::Win32::Graphics::Dxgi::{
    DXGIGetDebugInterface1, IDXGIInfoQueue, DXGI_DEBUG_ALL, DXGI_INFO_QUEUE_MESSAGE,
};
use windows::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleExA, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};
use windows::Win32::System::Threading::{CreateEventExW, WaitForSingleObjectEx, CREATE_EVENT};
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

/// Helper for fallible [`windows`] APIs that have an out-param with a default
/// value.
///
/// # Example
///
/// ```
/// let swap_chain_desc = try_out_param(|sd| unsafe { self.swap_chain.GetDesc1(sd) })?;
/// ```
pub fn try_out_param<T, F, E, O>(mut f: F) -> Result<T, E>
where
    T: Default,
    F: FnMut(&mut T) -> Result<O, E>,
{
    let mut t: T = Default::default();
    match f(&mut t) {
        Ok(_) => Ok(t),
        Err(e) => Err(e),
    }
}

/// Helper for fallible [`windows`] APIs that have an optional pointer
/// out-param.
///
/// # Example
///
/// ```
/// let dev: ID3D12Device =
///     try_out_ptr(|v| unsafe { D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_11_0, v) })
///         .expect("D3D12CreateDevice failed");
/// ```
pub fn try_out_ptr<T, F, E, O>(mut f: F) -> Result<T, E>
where
    F: FnMut(&mut Option<T>) -> Result<O, E>,
{
    let mut t: Option<T> = None;
    match f(&mut t) {
        Ok(_) => Ok(t.unwrap()),
        Err(e) => Err(e),
    }
}

/// Helper for fallible [`windows`] APIs that have an optional pointer
/// out-param and an optional pointer err-param.
///
/// # Example
///
/// ```
/// let blob: ID3DBlob = util::try_out_err_blob(|v, err_blob| {
///     D3D12SerializeRootSignature(
///         &root_signature_desc,
///         D3D_ROOT_SIGNATURE_VERSION_1_0,
///         v,
///         Some(err_blob),
///     )
/// })
/// .map_err(print_err_blob("Compiling vertex shader"))?;
/// ```
pub fn try_out_err_blob<T1, T2, F, E, O>(mut f: F) -> Result<T1, (E, T2)>
where
    F: FnMut(&mut Option<T1>, &mut Option<T2>) -> Result<O, E>,
{
    let mut t1: Option<T1> = None;
    let mut t2: Option<T2> = None;
    match f(&mut t1, &mut t2) {
        Ok(_) => Ok(t1.unwrap()),
        Err(e) => Err((e, t2.unwrap())),
    }
}

/// Helper for infallible APIs that have out-params, like OpenGL 3.
///
/// # Example
///
/// ```
/// let vertex_buffer = out_param(|x| unsafe { gl.GenBuffers(1, x) });
/// ```
pub fn out_param<T: Default, F>(f: F) -> T
where
    F: FnOnce(&mut T),
{
    let mut val = Default::default();
    f(&mut val);
    val
}

/// Use together with [`try_out_err_blob`] for printing Direct3D error blobs.
pub fn print_error_blob<D: Display, E>(msg: D) -> impl Fn((E, ID3DBlob)) -> E {
    move |(e, err_blob): (E, ID3DBlob)| {
        let buf_ptr = unsafe { err_blob.GetBufferPointer() } as *mut u8;
        let buf_size = unsafe { err_blob.GetBufferSize() };
        let s = unsafe { String::from_raw_parts(buf_ptr, buf_size, buf_size + 1) };
        error!("{msg}: {s}");
        e
    }
}

/// Enables the Direct3D12 debug interface.
///
/// It will not panic if the interface is not available. Call this from your
/// application before a DirectX 12 device is initialized. It could fail in
/// DirectX 12 host applications that will have initialized their device
/// already, but should not fail in other host applications.
pub fn enable_debug_interface() {
    let debug_interface: Result<ID3D12Debug, _> =
        try_out_ptr(|v| unsafe { D3D12GetDebugInterface(v) });

    match debug_interface {
        Ok(debug_interface) => unsafe { debug_interface.EnableDebugLayer() },
        Err(e) => {
            error!("Could not create debug interface: {e:?}")
        },
    }
}

/// Prints the DXGI debug messages on the debug trace. It is used internally for
/// error reporting, but can be used by clients. Has effect only after
/// [`enable_debug_interface`] has been called.
pub fn print_dxgi_debug_messages() {
    let Ok(diq): Result<IDXGIInfoQueue, _> = (unsafe { DXGIGetDebugInterface1(0) }) else {
        return;
    };

    let n = unsafe { diq.GetNumStoredMessages(DXGI_DEBUG_ALL) };
    for i in 0..n {
        let mut msg_len: usize = 0;
        unsafe { diq.GetMessage(DXGI_DEBUG_ALL, i, None, &mut msg_len as _).unwrap() };
        let diqm = vec![0u8; msg_len];
        let pdiqm = diqm.as_ptr() as *mut DXGI_INFO_QUEUE_MESSAGE;
        unsafe { diq.GetMessage(DXGI_DEBUG_ALL, i, Some(pdiqm), &mut msg_len as _).unwrap() };
        let diqm = unsafe { pdiqm.as_ref().unwrap() };
        debug!(
            "[DIQ] {}",
            String::from_utf8_lossy(unsafe {
                std::slice::from_raw_parts(diqm.pDescription, diqm.DescriptionByteLength - 1)
            })
        );
    }
    unsafe { diq.ClearStoredMessages(DXGI_DEBUG_ALL) };
}

/// Helper that returns width and height of a given
/// [`windows::Win32::Foundation::HWND`].
pub fn win_size(hwnd: HWND) -> (i32, i32) {
    let mut rect = RECT::default();
    unsafe { GetClientRect(hwnd, &mut rect).unwrap() };
    (rect.right - rect.left, rect.bottom - rect.top)
}

/// Returns the path of the current module.
pub fn get_dll_path() -> Option<PathBuf> {
    let mut hmodule = HMODULE(0);
    if let Err(e) = unsafe {
        GetModuleHandleExA(
            GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT | GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
            s!("DllMain"),
            &mut hmodule,
        )
    } {
        error!("get_dll_path: GetModuleHandleExA error: {e:?}");
        return None;
    }

    let mut sz_filename = [0u16; MAX_PATH as usize];
    let len = unsafe { GetModuleFileNameW(hmodule, &mut sz_filename) } as usize;

    Some(OsString::from_wide(&sz_filename[..len]).into())
}

/// Creates a [`D3D12_RESOURCE_BARRIER`].
///
/// Use this function and the associated [`drop_barrier`] for correctly managing
/// barrier resources.
///
/// RAII was not used due to the complicated signature of
/// [`windows::Win32::Graphics::Direct3D12::ID3D12GraphicsCommandList::ResourceBarrier`].
pub fn create_barrier(
    resource: &ID3D12Resource,
    before: D3D12_RESOURCE_STATES,
    after: D3D12_RESOURCE_STATES,
) -> D3D12_RESOURCE_BARRIER {
    D3D12_RESOURCE_BARRIER {
        Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
        Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
        Anonymous: D3D12_RESOURCE_BARRIER_0 {
            Transition: ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                pResource: ManuallyDrop::new(Some(resource.clone())),
                Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                StateBefore: before,
                StateAfter: after,
            }),
        },
    }
}

/// Drops a [`D3D12_RESOURCE_BARRIER`].
///
/// Use this function and the associated [`create_barrier`] for correctly
/// managing barrier resources.
///
/// RAII was not used due to the complicated signature of
/// [`windows::Win32::Graphics::Direct3D12::ID3D12GraphicsCommandList::ResourceBarrier`].
pub fn drop_barrier(barrier: D3D12_RESOURCE_BARRIER) {
    let transition = ManuallyDrop::into_inner(unsafe { barrier.Anonymous.Transition });
    let _ = ManuallyDrop::into_inner(transition.pResource);
}

/// Wrapper around [`windows::Win32::Graphics::Direct3D12::ID3D12Fence`].
pub struct Fence {
    fence: ID3D12Fence,
    value: AtomicU64,
    event: HANDLE,
}

impl Fence {
    /// Construct the fence.
    pub fn new(device: &ID3D12Device) -> windows::core::Result<Self> {
        let fence = unsafe { device.CreateFence(0, D3D12_FENCE_FLAG_NONE) }?;
        let value = AtomicU64::new(0);
        let event = unsafe { CreateEventExW(None, None, CREATE_EVENT(0), 0x1f0003) }?;

        Ok(Fence { fence, value, event })
    }

    /// Retrieve the underlying fence object to pass to the D3D12 APIs.
    pub fn fence(&self) -> &ID3D12Fence {
        &self.fence
    }

    /// Retrieve the current fence value.
    pub fn value(&self) -> u64 {
        self.value.load(Ordering::SeqCst)
    }

    /// Atomically increase the fence value.
    pub fn incr(&self) {
        self.value.fetch_add(1, Ordering::SeqCst);
    }

    /// Wait for completion of the fence.
    pub fn wait(&self) -> windows::core::Result<()> {
        let value = self.value();
        unsafe {
            if self.fence.GetCompletedValue() < value {
                self.fence.SetEventOnCompletion(value, self.event)?;
                WaitForSingleObjectEx(self.event, u32::MAX, false);
            }
        }

        Ok(())
    }
}
