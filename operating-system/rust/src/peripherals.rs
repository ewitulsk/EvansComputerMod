//! Peripheral integration module.
//!
//! This module provides access to peripheral host functions that are optionally
//! available when integration mods (like Advanced Peripherals integration) are installed.
//!
//! All functions in this module gracefully handle the case where the peripheral
//! is not available, returning appropriate error values.

/// Error codes returned by peripheral functions
pub mod error {
    /// Invalid arguments passed to the function
    pub const INVALID_ARGS: i32 = -1;
    /// Peripheral not available (integration mod not installed)
    pub const NOT_AVAILABLE: i32 = -2;
    /// Player not found
    pub const PLAYER_NOT_FOUND: i32 = -3;
}

/// Player Detector peripheral functions.
///
/// These functions are available when the Advanced Peripherals integration
/// mod is installed alongside the core mod.
pub mod player_detector {
    use super::*;
    
    // Buffer for reading player data
    static mut READ_BUFFER: [u8; 4096] = [0u8; 4096];
    
    extern "C" {
        /// Gets all online players as a JSON array.
        /// Returns bytes written to buffer, or negative error code.
        fn player_detector_get_online_players(buf_ptr: *mut u8, buf_len: usize) -> i32;
        
        /// Gets players within the specified range as a JSON array.
        /// Returns bytes written to buffer, or negative error code.
        fn player_detector_get_players_in_range(range: i32, buf_ptr: *mut u8, buf_len: usize) -> i32;
        
        /// Checks if a specific player is within range.
        /// Returns 1 if in range, 0 if not, or negative error code.
        fn player_detector_is_player_in_range(range: i32, name_ptr: *const u8, name_len: usize) -> i32;
        
        /// Gets the total count of online players.
        /// Returns the count, or negative error code.
        fn player_detector_get_player_count() -> i32;
        
        /// Gets a player's position and info as JSON.
        /// Returns bytes written to buffer, or negative error code.
        fn player_detector_get_player_pos(
            name_ptr: *const u8, 
            name_len: usize,
            buf_ptr: *mut u8, 
            buf_len: usize
        ) -> i32;
    }
    
    /// Checks if the player detector peripheral is available.
    ///
    /// Returns true if the Advanced Peripherals integration is installed.
    pub fn is_available() -> bool {
        unsafe {
            // Try to get player count - if it returns NOT_AVAILABLE, the peripheral isn't there
            player_detector_get_player_count() != error::NOT_AVAILABLE
        }
    }
    
    /// Gets all online players.
    ///
    /// Returns a vector of player names, or None if the peripheral is not available.
    ///
    /// # Example
    /// ```
    /// if let Some(players) = player_detector::get_online_players() {
    ///     for name in players {
    ///         terminal::println(&format!("Player: {}", name));
    ///     }
    /// }
    /// ```
    pub fn get_online_players() -> Option<Vec<String>> {
        unsafe {
            let result = player_detector_get_online_players(
                READ_BUFFER.as_mut_ptr(),
                READ_BUFFER.len()
            );
            
            if result < 0 {
                return None;
            }
            
            let json = std::str::from_utf8(&READ_BUFFER[..result as usize]).ok()?;
            parse_json_string_array(json)
        }
    }
    
    /// Gets players within the specified range of the terminal.
    ///
    /// # Arguments
    /// * `range` - Maximum distance in blocks. Use -1 for unlimited range.
    ///
    /// Returns a vector of player names, or None if the peripheral is not available.
    pub fn get_players_in_range(range: i32) -> Option<Vec<String>> {
        unsafe {
            let result = player_detector_get_players_in_range(
                range,
                READ_BUFFER.as_mut_ptr(),
                READ_BUFFER.len()
            );
            
            if result < 0 {
                return None;
            }
            
            let json = std::str::from_utf8(&READ_BUFFER[..result as usize]).ok()?;
            parse_json_string_array(json)
        }
    }
    
    /// Checks if a specific player is within range of the terminal.
    ///
    /// # Arguments
    /// * `range` - Maximum distance in blocks. Use -1 for unlimited range.
    /// * `name` - The player's name to check.
    ///
    /// Returns Some(true) if in range, Some(false) if not, None if unavailable or player not found.
    pub fn is_player_in_range(range: i32, name: &str) -> Option<bool> {
        unsafe {
            let result = player_detector_is_player_in_range(
                range,
                name.as_ptr(),
                name.len()
            );
            
            match result {
                1 => Some(true),
                0 => Some(false),
                _ => None,  // Error occurred
            }
        }
    }
    
    /// Gets the total count of online players.
    ///
    /// Returns the player count, or None if the peripheral is not available.
    pub fn get_player_count() -> Option<i32> {
        unsafe {
            let result = player_detector_get_player_count();
            if result < 0 {
                None
            } else {
                Some(result)
            }
        }
    }
    
    /// Player position and info.
    #[derive(Debug, Clone)]
    pub struct PlayerInfo {
        pub x: f64,
        pub y: f64,
        pub z: f64,
        pub dimension: String,
        pub yaw: f32,
        pub pitch: f32,
        pub health: f32,
        pub max_health: f32,
    }
    
    /// Gets a player's position and info.
    ///
    /// # Arguments
    /// * `name` - The player's name.
    ///
    /// Returns the player info, or None if not found or unavailable.
    pub fn get_player_pos(name: &str) -> Option<PlayerInfo> {
        unsafe {
            let result = player_detector_get_player_pos(
                name.as_ptr(),
                name.len(),
                READ_BUFFER.as_mut_ptr(),
                READ_BUFFER.len()
            );
            
            if result < 0 {
                return None;
            }
            
            let json = std::str::from_utf8(&READ_BUFFER[..result as usize]).ok()?;
            parse_player_info(json)
        }
    }
    
    // ==================== JSON Parsing Helpers ====================
    // Simple JSON parsing without external dependencies
    
    /// Parses a JSON array of strings: ["a", "b", "c"]
    pub fn parse_json_string_array(json: &str) -> Option<Vec<String>> {
        let json = json.trim();
        if !json.starts_with('[') || !json.ends_with(']') {
            return None;
        }
        
        let inner = &json[1..json.len()-1];
        if inner.trim().is_empty() {
            return Some(Vec::new());
        }
        
        let mut result = Vec::new();
        let mut in_string = false;
        let mut escape_next = false;
        let mut current = String::new();
        
        for c in inner.chars() {
            if escape_next {
                current.push(c);
                escape_next = false;
                continue;
            }
            
            match c {
                '\\' if in_string => {
                    escape_next = true;
                }
                '"' => {
                    if in_string {
                        result.push(current.clone());
                        current.clear();
                    }
                    in_string = !in_string;
                }
                _ if in_string => {
                    current.push(c);
                }
                _ => {
                    // Skip commas and whitespace outside strings
                }
            }
        }
        
        Some(result)
    }
    
    /// Parses a PlayerInfo from JSON
    fn parse_player_info(json: &str) -> Option<PlayerInfo> {
        // Simple extraction of numeric values from JSON
        let x = extract_f64(json, "\"x\":")?;
        let y = extract_f64(json, "\"y\":")?;
        let z = extract_f64(json, "\"z\":")?;
        let dimension = extract_string(json, "\"dimension\":")?.to_string();
        let yaw = extract_f64(json, "\"yaw\":").unwrap_or(0.0) as f32;
        let pitch = extract_f64(json, "\"pitch\":").unwrap_or(0.0) as f32;
        let health = extract_f64(json, "\"health\":").unwrap_or(20.0) as f32;
        let max_health = extract_f64(json, "\"maxHealth\":").unwrap_or(20.0) as f32;
        
        Some(PlayerInfo {
            x,
            y,
            z,
            dimension,
            yaw,
            pitch,
            health,
            max_health,
        })
    }
    
    /// Extracts a f64 value from JSON by key prefix
    fn extract_f64(json: &str, key: &str) -> Option<f64> {
        let idx = json.find(key)?;
        let start = idx + key.len();
        let rest = &json[start..];
        
        // Find end of number (next comma, brace, or end)
        let end = rest.find(|c: char| c == ',' || c == '}' || c == ']').unwrap_or(rest.len());
        let num_str = rest[..end].trim();
        
        num_str.parse().ok()
    }
    
    /// Extracts a string value from JSON by key prefix
    fn extract_string<'a>(json: &'a str, key: &str) -> Option<&'a str> {
        let idx = json.find(key)?;
        let start = idx + key.len();
        let rest = json[start..].trim_start();
        
        if !rest.starts_with('"') {
            return None;
        }
        
        let rest = &rest[1..];  // Skip opening quote
        let end = rest.find('"')?;  // Find closing quote
        
        Some(&rest[..end])
    }
}

/// Environment Detector peripheral functions.
pub mod environment_detector {
    use super::*;
    
    static mut READ_BUFFER: [u8; 8192] = [0u8; 8192];
    
    extern "C" {
        fn environment_get_biome(buf_ptr: *mut u8, buf_len: usize) -> i32;
        fn environment_get_time() -> i64;
        fn environment_get_moon_id() -> i32;
        fn environment_get_moon_name(buf_ptr: *mut u8, buf_len: usize) -> i32;
        fn environment_is_raining() -> i32;
        fn environment_is_thunder() -> i32;
        fn environment_is_sunny() -> i32;
        fn environment_get_dimension(buf_ptr: *mut u8, buf_len: usize) -> i32;
        fn environment_list_dimensions(buf_ptr: *mut u8, buf_len: usize) -> i32;
        fn environment_get_sky_light_level() -> i32;
        fn environment_get_block_light_level() -> i32;
        fn environment_get_day_light_level() -> i32;
        fn environment_is_slime_chunk() -> i32;
        fn environment_scan_entities(radius: i32, buf_ptr: *mut u8, buf_len: usize) -> i32;
    }
    
    pub fn is_available() -> bool {
        unsafe { environment_get_time() != error::NOT_AVAILABLE as i64 }
    }
    
    pub fn get_biome() -> Option<String> {
        unsafe {
            let result = environment_get_biome(READ_BUFFER.as_mut_ptr(), READ_BUFFER.len());
            if result < 0 { return None; }
            std::str::from_utf8(&READ_BUFFER[..result as usize]).ok().map(|s| s.to_string())
        }
    }
    
    pub fn get_time() -> Option<i64> {
        unsafe {
            let result = environment_get_time();
            if result < 0 { None } else { Some(result) }
        }
    }
    
    pub fn get_moon_id() -> Option<i32> {
        unsafe {
            let result = environment_get_moon_id();
            if result < 0 { None } else { Some(result) }
        }
    }
    
    pub fn get_moon_name() -> Option<String> {
        unsafe {
            let result = environment_get_moon_name(READ_BUFFER.as_mut_ptr(), READ_BUFFER.len());
            if result < 0 { return None; }
            std::str::from_utf8(&READ_BUFFER[..result as usize]).ok().map(|s| s.to_string())
        }
    }
    
    pub fn is_raining() -> Option<bool> {
        unsafe {
            match environment_is_raining() {
                1 => Some(true),
                0 => Some(false),
                _ => None,
            }
        }
    }
    
    pub fn is_thunder() -> Option<bool> {
        unsafe {
            match environment_is_thunder() {
                1 => Some(true),
                0 => Some(false),
                _ => None,
            }
        }
    }
    
    pub fn is_sunny() -> Option<bool> {
        unsafe {
            match environment_is_sunny() {
                1 => Some(true),
                0 => Some(false),
                _ => None,
            }
        }
    }
    
    pub fn get_dimension() -> Option<String> {
        unsafe {
            let result = environment_get_dimension(READ_BUFFER.as_mut_ptr(), READ_BUFFER.len());
            if result < 0 { return None; }
            std::str::from_utf8(&READ_BUFFER[..result as usize]).ok().map(|s| s.to_string())
        }
    }
    
    pub fn list_dimensions() -> Option<Vec<String>> {
        unsafe {
            let result = environment_list_dimensions(READ_BUFFER.as_mut_ptr(), READ_BUFFER.len());
            if result < 0 { return None; }
            let json = std::str::from_utf8(&READ_BUFFER[..result as usize]).ok()?;
            player_detector::parse_json_string_array(json)
        }
    }
    
    pub fn get_sky_light_level() -> Option<i32> {
        unsafe {
            let result = environment_get_sky_light_level();
            if result < 0 { None } else { Some(result) }
        }
    }
    
    pub fn get_block_light_level() -> Option<i32> {
        unsafe {
            let result = environment_get_block_light_level();
            if result < 0 { None } else { Some(result) }
        }
    }
    
    pub fn get_day_light_level() -> Option<i32> {
        unsafe {
            let result = environment_get_day_light_level();
            if result < 0 { None } else { Some(result) }
        }
    }
    
    pub fn is_slime_chunk() -> Option<bool> {
        unsafe {
            match environment_is_slime_chunk() {
                1 => Some(true),
                0 => Some(false),
                _ => None,
            }
        }
    }
    
    pub fn scan_entities(radius: i32) -> Option<String> {
        unsafe {
            let result = environment_scan_entities(radius, READ_BUFFER.as_mut_ptr(), READ_BUFFER.len());
            if result < 0 { return None; }
            std::str::from_utf8(&READ_BUFFER[..result as usize]).ok().map(|s| s.to_string())
        }
    }
}

/// Chat Box peripheral functions.
pub mod chat_box {
    use super::*;
    
    extern "C" {
        fn chat_send_message(msg_ptr: *const u8, msg_len: usize) -> i32;
        fn chat_send_message_to_player(player_ptr: *const u8, player_len: usize, 
                                        msg_ptr: *const u8, msg_len: usize) -> i32;
        fn chat_send_toast(player_ptr: *const u8, player_len: usize,
                           title_ptr: *const u8, title_len: usize,
                           msg_ptr: *const u8, msg_len: usize) -> i32;
    }
    
    pub fn is_available() -> bool {
        // Try sending empty message to check availability
        unsafe { chat_send_message("".as_ptr(), 0) != error::NOT_AVAILABLE }
    }
    
    pub fn send_message(message: &str) -> bool {
        unsafe { chat_send_message(message.as_ptr(), message.len()) > 0 }
    }
    
    pub fn send_message_to_player(player: &str, message: &str) -> bool {
        unsafe { 
            chat_send_message_to_player(
                player.as_ptr(), player.len(),
                message.as_ptr(), message.len()
            ) > 0
        }
    }
    
    pub fn send_toast(player: &str, title: &str, message: &str) -> bool {
        unsafe {
            chat_send_toast(
                player.as_ptr(), player.len(),
                title.as_ptr(), title.len(),
                message.as_ptr(), message.len()
            ) > 0
        }
    }
}

/// Geo Scanner peripheral functions.
pub mod geo_scanner {
    use super::*;
    
    static mut READ_BUFFER: [u8; 65536] = [0u8; 65536];  // Large buffer for scan results
    
    extern "C" {
        fn geo_scan(radius: i32, buf_ptr: *mut u8, buf_len: usize) -> i32;
        fn geo_chunk_analyze(buf_ptr: *mut u8, buf_len: usize) -> i32;
        fn geo_scan_cost(radius: i32) -> i32;
    }
    
    pub fn is_available() -> bool {
        unsafe { geo_scan_cost(1) != error::NOT_AVAILABLE }
    }
    
    pub fn scan(radius: i32) -> Option<String> {
        unsafe {
            let result = geo_scan(radius, READ_BUFFER.as_mut_ptr(), READ_BUFFER.len());
            if result < 0 { return None; }
            std::str::from_utf8(&READ_BUFFER[..result as usize]).ok().map(|s| s.to_string())
        }
    }
    
    pub fn chunk_analyze() -> Option<String> {
        unsafe {
            let result = geo_chunk_analyze(READ_BUFFER.as_mut_ptr(), READ_BUFFER.len());
            if result < 0 { return None; }
            std::str::from_utf8(&READ_BUFFER[..result as usize]).ok().map(|s| s.to_string())
        }
    }
    
    pub fn scan_cost(radius: i32) -> Option<i32> {
        unsafe {
            let result = geo_scan_cost(radius);
            if result < 0 { None } else { Some(result) }
        }
    }
}

/// Block Reader peripheral functions.
pub mod block_reader {
    use super::*;
    
    static mut READ_BUFFER: [u8; 8192] = [0u8; 8192];
    
    extern "C" {
        fn block_reader_get_name(buf_ptr: *mut u8, buf_len: usize) -> i32;
        fn block_reader_get_data(buf_ptr: *mut u8, buf_len: usize) -> i32;
        fn block_reader_get_states(buf_ptr: *mut u8, buf_len: usize) -> i32;
        fn block_reader_is_tile_entity() -> i32;
    }
    
    pub fn is_available() -> bool {
        unsafe { block_reader_is_tile_entity() != error::NOT_AVAILABLE }
    }
    
    pub fn get_name() -> Option<String> {
        unsafe {
            let result = block_reader_get_name(READ_BUFFER.as_mut_ptr(), READ_BUFFER.len());
            if result < 0 { return None; }
            std::str::from_utf8(&READ_BUFFER[..result as usize]).ok().map(|s| s.to_string())
        }
    }
    
    pub fn get_data() -> Option<String> {
        unsafe {
            let result = block_reader_get_data(READ_BUFFER.as_mut_ptr(), READ_BUFFER.len());
            if result < 0 { return None; }
            std::str::from_utf8(&READ_BUFFER[..result as usize]).ok().map(|s| s.to_string())
        }
    }
    
    pub fn get_states() -> Option<String> {
        unsafe {
            let result = block_reader_get_states(READ_BUFFER.as_mut_ptr(), READ_BUFFER.len());
            if result < 0 { return None; }
            std::str::from_utf8(&READ_BUFFER[..result as usize]).ok().map(|s| s.to_string())
        }
    }
    
    pub fn is_tile_entity() -> Option<bool> {
        unsafe {
            match block_reader_is_tile_entity() {
                1 => Some(true),
                0 => Some(false),
                _ => None,
            }
        }
    }
}

/// Energy Detector peripheral functions.
pub mod energy_detector {
    use super::*;
    
    extern "C" {
        fn energy_get_transfer_rate() -> i32;
        fn energy_get_transfer_rate_limit() -> i32;
        fn energy_set_transfer_rate_limit(rate: i32) -> i32;
    }
    
    pub fn is_available() -> bool {
        unsafe { energy_get_transfer_rate() != error::NOT_AVAILABLE }
    }
    
    pub fn get_transfer_rate() -> Option<i32> {
        unsafe {
            let result = energy_get_transfer_rate();
            if result < 0 { None } else { Some(result) }
        }
    }
    
    pub fn get_transfer_rate_limit() -> Option<i32> {
        unsafe {
            let result = energy_get_transfer_rate_limit();
            if result < 0 { None } else { Some(result) }
        }
    }
    
    pub fn set_transfer_rate_limit(rate: i32) -> bool {
        unsafe { energy_set_transfer_rate_limit(rate) >= 0 }
    }
}

/// NBT Storage peripheral functions.
pub mod nbt_storage {
    use super::*;
    
    static mut READ_BUFFER: [u8; 65536] = [0u8; 65536];
    
    extern "C" {
        fn nbt_read(buf_ptr: *mut u8, buf_len: usize) -> i32;
        fn nbt_write_json(json_ptr: *const u8, json_len: usize) -> i32;
    }
    
    pub fn is_available() -> bool {
        // Check by trying to read - empty storage returns empty result, not error
        unsafe {
            let result = nbt_read(READ_BUFFER.as_mut_ptr(), READ_BUFFER.len());
            result != error::NOT_AVAILABLE
        }
    }
    
    pub fn read() -> Option<String> {
        unsafe {
            let result = nbt_read(READ_BUFFER.as_mut_ptr(), READ_BUFFER.len());
            if result < 0 { return None; }
            std::str::from_utf8(&READ_BUFFER[..result as usize]).ok().map(|s| s.to_string())
        }
    }
    
    pub fn write_json(json: &str) -> bool {
        unsafe { nbt_write_json(json.as_ptr(), json.len()) > 0 }
    }
}
