//! This module contains logic for filtering windows messages.

use bitflags::bitflags;
use windows::Win32::UI::WindowsAndMessaging::*;

bitflags! {
    /// Bitflag for specifying types of window message to be filtered.
    ///
    /// Return this on [`ImguiRenderLoop::message_filter`](crate::ImguiRenderLoop::message_filter)
    /// to filter certain types of window message.
    ///
    /// You can use bitwise-or to combine multiple flags.
    ///
    /// Example usage:
    /// ```no_run
    /// // impl ImguiRenderLoop for ...
    /// fn message_filter(&self, _io: &Io) -> MessageFilter {
    ///     if self.visible {
    ///         MessageFilter::InputAll | MessageFilter::WindowClose
    ///     } else {
    ///         MessageFilter::empty()
    ///     }
    /// }
    /// ```
    #[repr(transparent)]
    pub struct MessageFilter: u32 {
        /// Blocks keyboard input event messages.
        const InputKeyboard = 1u32 << 0;
        /// Blocks mouse input event message.
        const InputMouse = 1u32 << 1;
        /// Blocks raw input event messages.
        const InputRaw = 1u32 << 2;

        /// Blocks window gain/lose focus event messages.
        const WindowFocus = 1u32 << 8;
        /// Blocks window control event messages
        /// like move, resize, minimize, etc.
        const WindowControl = 1u32 << 9;
        /// Blocks window close messages.
        const WindowClose = 1u32 << 10;

        /// Blocks messages ID from 0 to `WM_USER` - 1
        /// (the range for system-defined messages).
        const RangeSystemDefined = 1u32 << 28;
        /// Blocks messages ID from `WM_USER` to `WM_APP` - 1
        /// (the range for private window classes like form button).
        const RangePrivateReserved = 1u32 << 29;
        /// Blocks messages ID from `WM_APP` to 0xBFFF
        /// (the range for internal use of user application).
        const RangeAppPrivate = 1u32 << 30;
        /// Blocks messages ID from 0xC000 to 0xFFFF
        /// (the range for registered use between user applications).
        const RangeAppRegistered = 1u32 << 31;

        /// Blocks keyboard, mouse, raw input messages.
        const InputAll = Self::InputKeyboard.bits() | Self::InputMouse.bits() | Self::InputRaw.bits();
        /// Blocks window focus, control, close messages.
        const WindowAll = Self::WindowFocus.bits() | Self::WindowControl.bits() | Self::WindowClose.bits();
    }
}

impl MessageFilter {
    /// Check whether the message ID is blocked by this filter
    pub(crate) fn is_blocking(&self, message_id: u32) -> bool {
        if match message_id {
            0x0000..=0x03FF => self.contains(Self::RangeSystemDefined),
            WM_USER..=0x7FFF => self.contains(Self::RangePrivateReserved),
            WM_APP..=0xBFFF => self.contains(Self::RangeAppPrivate),
            0xC000..=0xFFFF => self.contains(Self::RangeAppRegistered),
            0x10000.. => return false,
        } {
            return true;
        }

        match message_id {
            WM_KEYFIRST..=WM_KEYLAST => self.contains(Self::InputKeyboard),
            WM_MOUSEFIRST..=WM_MOUSELAST => self.contains(Self::InputMouse),
            WM_INPUT => self.contains(Self::InputRaw),

            WM_MOUSEACTIVATE | WM_ACTIVATEAPP | WM_ACTIVATE | WM_SETFOCUS | WM_KILLFOCUS
            | WM_ENABLE => self.contains(Self::WindowFocus),

            WM_SYSCOMMAND | WM_GETMINMAXINFO | WM_ENTERSIZEMOVE | WM_EXITSIZEMOVE
            | WM_WINDOWPOSCHANGING | WM_WINDOWPOSCHANGED | WM_SHOWWINDOW | WM_MOVING | WM_MOVE
            | WM_SIZING | WM_SIZE => self.contains(Self::WindowControl),

            WM_CLOSE => self.contains(Self::WindowClose),
            _ => false,
        }
    }
}
