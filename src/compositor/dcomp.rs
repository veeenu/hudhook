use windows::core::Result;
use windows::Win32::Foundation::{BOOL, HWND};
use windows::Win32::Graphics::Direct3D12::ID3D12Resource;
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};

// Holds and manages the lifetimes for the DirectComposition data structures.
pub struct Compositor {
    dcomp_dev: IDCompositionDevice,
    _dcomp_target: IDCompositionTarget,
    root_visual: IDCompositionVisual,
}

impl Compositor {
    pub fn new(target_hwnd: HWND) -> Result<Self> {
        let dcomp_dev: IDCompositionDevice = unsafe { DCompositionCreateDevice(None) }?;
        let dcomp_target = unsafe { dcomp_dev.CreateTargetForHwnd(target_hwnd, BOOL::from(true)) }?;

        let root_visual = unsafe { dcomp_dev.CreateVisual() }?;
        unsafe { dcomp_target.SetRoot(&root_visual) }?;
        unsafe { dcomp_dev.Commit() }?;

        Ok(Self { dcomp_dev, _dcomp_target: dcomp_target, root_visual })
    }

    pub fn composite(&self, resource: ID3D12Resource) -> Result<()> {
        unsafe {
            self.root_visual.SetContent(resource)?;
            self.dcomp_dev.Commit()?;
        }

        Ok(())
    }
}
