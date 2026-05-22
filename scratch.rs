fn main() {
    let s = "E:\\SteamLibrary\\steamapps\\common\\The Lord of the Rings Return to Moriaâ„¢";
    let bytes: Vec<u8> = s.chars().map(|c| {
        // basic windows-1252 to byte reverse mapping
        match c {
            'â' => 0xE2,
            '„' => 0x84,
            '¢' => 0xA2,
            _ => c as u8,
        }
    }).collect();
    if let Ok(fixed) = String::from_utf8(bytes) {
        println!("Fixed: {}", fixed);
    }
}
