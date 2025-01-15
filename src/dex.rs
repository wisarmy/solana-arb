use bitflags::bitflags;

bitflags! {
    #[derive(Debug, Clone, Copy)]
    pub struct Dex: u8 {
        const RAYDIUM = 0b0000_0001;
        const METEORA_DLMM = 0b0000_0010;
        const WHIRLPOOL = 0b0000_0100;
        const ALL = Self::RAYDIUM.bits() | Self::METEORA_DLMM.bits() | Self::WHIRLPOOL.bits();
    }
}

impl ToString for Dex {
    fn to_string(&self) -> String {
        let mut dexes = Vec::new();

        if self.contains(Dex::RAYDIUM) {
            dexes.push("Raydium");
        }
        if self.contains(Dex::METEORA_DLMM) {
            dexes.push("Meteora DLMM");
        }
        if self.contains(Dex::WHIRLPOOL) {
            dexes.push("Whirlpool");
        }

        dexes.join(",")
    }
}
