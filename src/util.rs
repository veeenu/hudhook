use std::mem::{self, ManuallyDrop};
use std::sync::atomic::{AtomicU64, Ordering};

use windows::Win32::Foundation::{HANDLE, HWND, RECT};
use windows::Win32::Graphics::Direct3D12::{
    ID3D12Device, ID3D12Fence, ID3D12Resource, D3D12_FENCE_FLAG_NONE, D3D12_RESOURCE_BARRIER,
    D3D12_RESOURCE_BARRIER_0, D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
    D3D12_RESOURCE_BARRIER_FLAG_NONE, D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
    D3D12_RESOURCE_STATES, D3D12_RESOURCE_TRANSITION_BARRIER,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObjectEx};
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

/// Helper that returns width and height of a given
/// [`windows::Win32::Foundation::HWND`].
pub fn win_size(hwnd: HWND) -> (i32, i32) {
    let mut rect = RECT::default();
    unsafe { GetClientRect(hwnd, &mut rect).unwrap() };
    (rect.right - rect.left, rect.bottom - rect.top)
}

// RAII wrapper around a [`std::mem::ManuallyDrop`] for a D3D12 resource
// barrier.
#[deprecated]
pub struct Barrier([D3D12_RESOURCE_BARRIER; 1]);

#[deprecated]
impl Barrier {
    pub fn new(
        buf: ID3D12Resource,
        before: D3D12_RESOURCE_STATES,
        after: D3D12_RESOURCE_STATES,
    ) -> Self {
        let transition_barrier = ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
            pResource: ManuallyDrop::new(Some(buf)),
            Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
            StateBefore: before,
            StateAfter: after,
        });

        let barrier = D3D12_RESOURCE_BARRIER {
            Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            Anonymous: D3D12_RESOURCE_BARRIER_0 { Transition: transition_barrier },
        };

        Self::from(barrier)
    }

    pub fn into_inner(mut self) -> D3D12_RESOURCE_BARRIER {
        mem::take(&mut self.0[0])
    }
}

impl From<D3D12_RESOURCE_BARRIER> for Barrier {
    fn from(value: D3D12_RESOURCE_BARRIER) -> Self {
        Self([value])
    }
}

impl AsRef<[D3D12_RESOURCE_BARRIER]> for Barrier {
    fn as_ref(&self) -> &[D3D12_RESOURCE_BARRIER] {
        &self.0
    }
}

impl Drop for Barrier {
    fn drop(&mut self) {
        let barrier = mem::take(&mut self.0);
        for barrier in barrier {
            let transition = ManuallyDrop::into_inner(unsafe { barrier.Anonymous.Transition });
            let _ = ManuallyDrop::into_inner(transition.pResource);
        }
    }
}

pub(crate) fn create_barrier(
    resource: &ID3D12Resource,
    before: D3D12_RESOURCE_STATES,
    after: D3D12_RESOURCE_STATES,
) -> D3D12_RESOURCE_BARRIER {
    D3D12_RESOURCE_BARRIER {
        Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
        Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
        Anonymous: D3D12_RESOURCE_BARRIER_0 {
            Transition: ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                pResource: unsafe { mem::transmute_copy(resource) },
                Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                StateBefore: before,
                StateAfter: after,
            }),
        },
    }
}

pub(crate) fn drop_barrier(barrier: D3D12_RESOURCE_BARRIER) {
    ManuallyDrop::into_inner(unsafe { barrier.Anonymous.Transition });
}

pub(crate) struct Fence {
    fence: ID3D12Fence,
    value: AtomicU64,
    event: HANDLE,
}

impl Fence {
    pub(crate) fn new(device: &ID3D12Device) -> windows::core::Result<Self> {
        let fence = unsafe { device.CreateFence(0, D3D12_FENCE_FLAG_NONE) }?;
        let value = AtomicU64::new(0);
        let event = unsafe { CreateEventW(None, false, true, None) }?;

        Ok(Fence { fence, value, event })
    }

    pub(crate) fn fence(&self) -> &ID3D12Fence {
        &self.fence
    }

    pub(crate) fn value(&self) -> u64 {
        self.value.load(Ordering::SeqCst)
    }

    pub(crate) fn incr(&self) {
        self.value.fetch_add(1, Ordering::SeqCst);
    }

    pub(crate) fn wait(&self) -> windows::core::Result<()> {
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
