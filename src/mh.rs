//! Thin FFI wrapper around [`minhook`](https://github.com/TsudaKageyu/minhook).
#![allow(dead_code, non_snake_case, non_camel_case_types, missing_docs)]

use std::ffi::c_void;
use std::ptr::null_mut;

use tracing::error;

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
    pub fn ok_context(self, context: &str) -> Result<(), MH_STATUS> {
        if self == MH_STATUS::MH_OK {
            Ok(())
        } else {
            error!("{context}: {self:?}");
            Err(self)
        }
    }

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
    ///
    /// Most definitely undefined behavior.
    pub unsafe fn new(addr: *mut c_void, hook_impl: *mut c_void) -> Result<Self, MH_STATUS> {
        let mut trampoline = null_mut();
        MH_CreateHook(addr, hook_impl, &mut trampoline).ok_context("MH_CreateHook")?;

        Ok(Self { addr, hook_impl, trampoline })
    }

    pub fn trampoline(&self) -> *mut c_void {
        self.trampoline
    }

    /// # Safety
    ///
    /// Most definitely undefined behavior.
    pub unsafe fn queue_enable(&self) -> Result<(), MH_STATUS> {
        MH_QueueEnableHook(self.addr).ok_context("MH_QueueEnableHook")
    }

    /// # Safety
    ///
    /// Most definitely undefined behavior.
    pub unsafe fn queue_disable(&self) -> Result<(), MH_STATUS> {
        MH_QueueDisableHook(self.addr).ok_context("MH_QueueDisableHook")
    }
}
