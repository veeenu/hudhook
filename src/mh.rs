#![allow(dead_code, non_snake_case, non_camel_case_types)]

use std::ffi::c_void;
use std::ptr::null_mut;

use log::*;

#[allow(non_camel_case_types)]
#[must_use]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MH_STATUS {
    /// Unknown error. Should not be returned.
    MH_UNKNOWN = -1,
    /// Successful.
    MH_OK = 0,
    /// MinHook is already initialized.
    MH_ERROR_ALREADY_INITIALIZED,
    /// MinHook is not initialized yet, or already uninitialized.
    MH_ERROR_NOT_INITIALIZED,
    /// The hook for the specified target function is already created.
    MH_ERROR_ALREADY_CREATED,
    /// The hook for the specified target function is not created yet.
    MH_ERROR_NOT_CREATED,
    /// The hook for the specified target function is already enabled.
    MH_ERROR_ENABLED,
    /// The hook for the specified target function is not enabled yet, or
    /// already disabled.
    MH_ERROR_DISABLED,
    /// The specified pointer is invalid. It points the address of non-allocated
    /// and/or non-executable region.
    MH_ERROR_NOT_EXECUTABLE,
    /// The specified target function cannot be hooked.
    MH_ERROR_UNSUPPORTED_FUNCTION,
    /// Failed to allocate memory.
    MH_ERROR_MEMORY_ALLOC,
    /// Failed to change the memory protection.
    MH_ERROR_MEMORY_PROTECT,
    /// The specified module is not loaded.
    MH_ERROR_MODULE_NOT_FOUND,
    /// The specified function is not found.
    MH_ERROR_FUNCTION_NOT_FOUND,
}

extern "system" {
    pub fn MH_Initialize() -> MH_STATUS;
    pub fn MH_Uninitialize() -> MH_STATUS;
    pub fn MH_CreateHook(
        pTarget: *mut c_void,
        pDetour: *mut c_void,
        ppOriginal: *mut *mut c_void,
    ) -> MH_STATUS;
    pub fn MH_EnableHook(pTarget: *mut c_void) -> MH_STATUS;
    pub fn MH_QueueEnableHook(pTarget: *mut c_void) -> MH_STATUS;
    pub fn MH_DisableHook(pTarget: *mut c_void) -> MH_STATUS;
    pub fn MH_QueueDisableHook(pTarget: *mut c_void) -> MH_STATUS;
    pub fn MH_ApplyQueued() -> MH_STATUS;
}

impl MH_STATUS {
    pub fn ok(self) -> Result<(), MH_STATUS> {
        if self == MH_STATUS::MH_OK {
            Ok(())
        } else {
            Err(self)
        }
    }
}

/// Structure that holds original address, hook function address, and trampoline
/// address for a given hook.
pub struct MhHook {
    addr: *mut c_void,
    hook_impl: *mut c_void,
    trampoline: *mut c_void,
}

impl MhHook {
    /// # Safety
    pub unsafe fn new(addr: *mut c_void, hook_impl: *mut c_void) -> Result<Self, MH_STATUS> {
        let mut trampoline = null_mut();
        let status = MH_CreateHook(addr, hook_impl, &mut trampoline);
        debug!("MH_CreateHook: {:?}", status);

        status.ok()?;

        Ok(Self { addr, hook_impl, trampoline })
    }

    pub fn trampoline(&self) -> *mut c_void {
        self.trampoline
    }

    unsafe fn queue_enable(&self) {
        let status = MH_QueueEnableHook(self.hook_impl);
        debug!("MH_QueueEnableHook: {:?}", status);
    }

    unsafe fn queue_disable(&self) {
        let status = MH_QueueDisableHook(self.hook_impl);
        debug!("MH_QueueDisableHook: {:?}", status);
    }
}

/// Wrapper for a queue of hooks to be applied via Minhook.
pub struct MhHooks(Vec<MhHook>);
unsafe impl Send for MhHooks {}
unsafe impl Sync for MhHooks {}

impl MhHooks {
    pub fn new<T: IntoIterator<Item = MhHook>>(hooks: T) -> Result<Self, MH_STATUS> {
        Ok(MhHooks(hooks.into_iter().collect::<Vec<_>>()))
    }

    pub fn apply(&self) {
        unsafe { MhHooks::apply_hooks(&self.0) };
    }

    pub fn unapply(&self) {
        unsafe { MhHooks::unapply_hooks(&self.0) };
        let status = unsafe { MH_Uninitialize() };
        debug!("MH_Uninitialize: {:?}", status);
    }

    unsafe fn apply_hooks(hooks: &[MhHook]) {
        for hook in hooks {
            let status = MH_QueueEnableHook(hook.addr);
            debug!("MH_QueueEnable: {:?}", status);
        }
        let status = MH_ApplyQueued();
        debug!("MH_ApplyQueued: {:?}", status);
    }

    unsafe fn unapply_hooks(hooks: &[MhHook]) {
        for hook in hooks {
            let status = MH_QueueDisableHook(hook.addr);
            debug!("MH_QueueDisable: {:?}", status);
        }
        let status = MH_ApplyQueued();
        debug!("MH_ApplyQueued: {:?}", status);
    }
}

impl Drop for MhHooks {
    fn drop(&mut self) {
        // self.unapply();
    }
}
