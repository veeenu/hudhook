#![allow(dead_code)]

use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::LazyLock;

use minhook_raw::*;
use tracing::debug;

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
        static INIT_CELL: LazyLock<()> = LazyLock::new(|| {
            let status = unsafe { MH_Initialize() };
            debug!("MH_Initialize: {:?}", status);
            if status != MH_STATUS::MH_OK {
                panic!("Couldn't initialize hooks");
            }
        });

        LazyLock::force(&INIT_CELL);

        let mut trampoline = null_mut();
        let status = MH_CreateHook(addr, hook_impl, &mut trampoline);
        debug!("MH_CreateHook: {:?}", status);

        if status != MH_STATUS::MH_OK {
            panic!("Couldn't create hook")
        }

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
