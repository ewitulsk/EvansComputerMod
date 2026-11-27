//! Reaching the player who is using the computer, outside the game
//! (abi/child-abi.toml `[imports.player]`).

#[cfg(target_arch = "wasm32")]
extern "C" {
    fn player_link(url_ptr: i32, url_len: i32, text_ptr: i32, text_len: i32) -> i32;
}

/// Why a link could not be shown.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LinkError {
    /// This host cannot show links (the simulator).
    Unsupported,
    /// Nobody is using the computer.
    NoPlayer,
    /// The server only shows links to the package repository's own pages.
    NotAllowed,
}

/// Put a clickable link in the chat of the player using this computer; clicking
/// it opens their real browser (Minecraft asks them first). The server only
/// accepts addresses of the package repository it is configured for, so a
/// program cannot use this to send players anywhere else.
pub fn show_link(url: &str, text: &str) -> Result<(), LinkError> {
    #[cfg(target_arch = "wasm32")]
    {
        match unsafe { player_link(url.as_ptr() as i32, url.len() as i32, text.as_ptr() as i32, text.len() as i32) } {
            0 => Ok(()),
            -4 => Err(LinkError::NoPlayer),
            -13 => Err(LinkError::NotAllowed),
            _ => Err(LinkError::Unsupported),
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (url, text);
        Err(LinkError::Unsupported)
    }
}
