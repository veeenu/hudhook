//! General-purpose utilities. These are used across the [`crate`] but have
//! proven useful in client code as well.

use std::ffi::{c_void, OsString};
use std::fmt::Display;
use std::mem::{size_of, ManuallyDrop};
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::{RwLock, RwLockReadGuard};
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
use windows::Win32::System::Memory::{
    VirtualQuery, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE,
    PAGE_PROTECTION_FLAGS, PAGE_READONLY, PAGE_READWRITE,
};
use windows::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
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

/// Returns a slice of **up to** `limit` elements of type `T` starting at `ptr`.
///
/// If the memory protection of some pages in this region prevents reading from
/// it, the slice is truncated to the first `N` consecutive readable elements.
///
/// # Safety
///
/// - `ptr` must not be a null pointer and must be properly aligned.
/// - Ignoring memory protection, the memory at `ptr` must be valid for at least
///   `limit` elements of type `T` (see [`std::slice::from_raw_parts`]).
pub unsafe fn readable_region<T>(ptr: *const T, limit: usize) -> &'static [T] {
    /// Check if the page pointed to by `ptr` is readable.
    unsafe fn is_readable(
        ptr: *const c_void,
        memory_basic_info: &mut MEMORY_BASIC_INFORMATION,
    ) -> bool {
        // If the page protection has any of these flags set, we can read from it
        const PAGE_READABLE: PAGE_PROTECTION_FLAGS = PAGE_PROTECTION_FLAGS(
            PAGE_READONLY.0 | PAGE_READWRITE.0 | PAGE_EXECUTE_READ.0 | PAGE_EXECUTE_READWRITE.0,
        );

        (unsafe {
            VirtualQuery(Some(ptr), memory_basic_info, size_of::<MEMORY_BASIC_INFORMATION>())
        } != 0)
            && (memory_basic_info.Protect & PAGE_READABLE).0 != 0
    }

    // This is probably 0x1000 (4096) bytes
    let page_size_bytes = {
        let mut system_info = SYSTEM_INFO::default();
        unsafe { GetSystemInfo(&mut system_info) };
        system_info.dwPageSize as usize
    };
    let page_align_mask = page_size_bytes - 1;

    // Calculate the starting address of the first and last pages that need to be
    // readable in order to read `limit` elements of type `T` from `ptr`
    let first_page_addr = (ptr as usize) & !page_align_mask;
    let last_page_addr = (ptr as usize + (limit * size_of::<T>()) - 1) & !page_align_mask;

    let mut memory_basic_info = MEMORY_BASIC_INFORMATION::default();
    for page_addr in (first_page_addr..=last_page_addr).step_by(page_size_bytes) {
        if unsafe { is_readable(page_addr as _, &mut memory_basic_info) } {
            continue;
        }

        // If this page is not readable, we can read from `ptr`
        // up to (not including) the start of this page
        //
        // Note: `page_addr` can be less than `ptr` if `ptr` is not page-aligned
        let num_readable = page_addr.saturating_sub(ptr as usize) / size_of::<T>();

        // SAFETY:
        // - `ptr` is a valid pointer to `limit` elements of type `T`
        // - `num_readable` is always less than or equal to `limit`
        return std::slice::from_raw_parts(ptr, num_readable);
    }

    // SAFETY:
    // - `ptr` is a valid pointer to `limit` elements of type `T` and is properly
    //   aligned
    std::slice::from_raw_parts(ptr, limit)
}

/// Implements a barrier to coordinate ejection of hooks
///
/// # Usave
/// - Hooked functions should call and maintain a guard from
///   `acquire_ejection_guard()` while they are in progress.
/// - Ejecting code should call `hudhook.unapply()` to ensure that no more
///   ejection guards will be acquired and then call `wait_for_all_blocks()` to
///   allow all hooks to exit before calling `FreeLibraryAndExitThread()`
///
/// This is implemented with a RwLock which allows us to have multiple
/// ejection guards in place without blocking each other, and then wait
/// for all the guards to complete before ejecting.
pub struct HookEjectionBarrier(RwLock<()>);
impl HookEjectionBarrier {
    /// Construct a new ejection barrier
    pub const fn new() -> Self {
        Self(RwLock::new(()))
    }

    /// Acquire a guard to prevent ejection while the guard exists
    ///
    /// Multiple guards can be acquired simultaneously and do not block
    /// each other.
    pub fn acquire_ejection_guard(&self) -> RwLockReadGuard<'_, ()> {
        self.0.read()
    }

    /// Wait for ejection to be safe.
    ///
    /// All ejection guards will be awaited before continuing. After this
    /// is called `acquire_ejection_guard()` should not be called again.
    pub fn wait_for_all_guards(&self) {
        // Note: We immediately drop the write lock once acquired, we just
        // need to ensure that all read locks have also been dropped.
        let _wait_guard = self.0.write();
    }
}

impl Default for HookEjectionBarrier {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use windows::Win32::System::Memory::{VirtualAlloc, VirtualProtect, MEM_COMMIT, PAGE_NOACCESS};

    use super::*;

    #[test]
    fn test_readable_region() -> windows::core::Result<()> {
        const PAGE_SIZE: usize = 0x1000;

        let region = unsafe { VirtualAlloc(None, 2 * PAGE_SIZE, MEM_COMMIT, PAGE_READWRITE) };
        if region.is_null() {
            return Err(windows::core::Error::from_win32());
        }

        // Make the second page unreadable
        let mut old_protect = PAGE_PROTECTION_FLAGS::default();
        unsafe {
            VirtualProtect(
                (region as usize + PAGE_SIZE) as _,
                PAGE_SIZE,
                PAGE_NOACCESS,
                &mut old_protect,
            )
        }?;
        assert_eq!(old_protect, PAGE_READWRITE);

        let slice = unsafe { readable_region::<u8>(region as _, PAGE_SIZE) };
        assert_eq!(slice.len(), PAGE_SIZE);

        let slice = unsafe { readable_region::<u8>(region as _, PAGE_SIZE + 1) };
        assert_eq!(slice.len(), PAGE_SIZE);

        let slice = unsafe { readable_region::<u8>((region as usize + PAGE_SIZE) as _, 1) };
        assert!(slice.is_empty());

        let slice = unsafe { readable_region::<u8>((region as usize + PAGE_SIZE - 1) as _, 2) };
        assert_eq!(slice.len(), 1);

        Ok(())
    }
}
