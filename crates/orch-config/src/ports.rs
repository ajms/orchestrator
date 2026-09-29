use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PortBlock {
    pub base: u16,
    pub size: u16,
}

impl PortBlock {
    fn end(self) -> u32 {
        u32::from(self.base) + u32::from(self.size)
    }

    pub fn overlaps(self, other: PortBlock) -> bool {
        u32::from(self.base) < other.end() && u32::from(other.base) < self.end()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortRange {
    pub start: u16,
    pub end: u16,
    pub block_size: u16,
}

impl Default for PortRange {
    fn default() -> Self {
        Self {
            start: 20000,
            end: 29999,
            block_size: 10,
        }
    }
}

impl PortRange {
    pub fn blocks(&self) -> impl Iterator<Item = PortBlock> + use<> {
        let (start, end, size) = (
            u32::from(self.start),
            u32::from(self.end),
            u32::from(self.block_size),
        );
        let count = match size {
            0 => 0,
            _ => (end + 1).saturating_sub(start) / size,
        };
        (0..count).map(move |index| PortBlock {
            base: (start + index * size) as u16,
            size: size as u16,
        })
    }
}
