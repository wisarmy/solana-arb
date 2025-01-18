use bitflags::bitflags;

bitflags! {
    #[derive(Debug, Clone, Copy)]
    pub struct Dex: u8 {
        const RAYDIUM = 0b0000_0001;
        const METEORA_DLMM = 0b0000_0010;
        const METEORA = 0b0000_0100;
        const WHIRLPOOL = 0b0000_1000;
        const PHOENIX = 0b0001_0000;
        const ALL = Self::RAYDIUM.bits() | Self::METEORA_DLMM.bits() | Self::WHIRLPOOL.bits() | Self::PHOENIX.bits();
    }
}

impl Dex {
    pub fn exclude(&self, other: &Dex) -> Self {
        Self::from_bits_truncate(self.bits() & !other.bits())
    }

    // Vec to Dex
    pub fn from_vec(v: Vec<&str>) -> Self {
        let mut dex = Dex::empty();
        for d in v {
            match d {
                "Raydium" => dex |= Dex::RAYDIUM,
                "Meteora DLMM" => dex |= Dex::METEORA_DLMM,
                "Meteora" => dex |= Dex::METEORA,
                "Whirlpool" => dex |= Dex::WHIRLPOOL,
                "Phoenix" => dex |= Dex::PHOENIX,
                _ => {}
            }
        }
        dex
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
        if self.contains(Dex::METEORA) {
            dexes.push("Meteora");
        }
        if self.contains(Dex::WHIRLPOOL) {
            dexes.push("Whirlpool");
        }
        if self.contains(Dex::PHOENIX) {
            dexes.push("Phoenix");
        }

        dexes.join(",")
    }
}
