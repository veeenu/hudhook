use windows::core::Result;
use windows::Win32::{
    Foundation::{BOOL, HWND},
    Graphics::{
        DirectComposition::{
            DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
        },
        Dxgi::IDXGISwapChain3,
    },
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

    pub fn render(&self, swap_chain: &IDXGISwapChain3) -> Result<()> {
        unsafe {
            self.root_visual.SetContent(swap_chain)?;
            self.dcomp_dev.Commit()?;
        }

        Ok(())
    }
}
