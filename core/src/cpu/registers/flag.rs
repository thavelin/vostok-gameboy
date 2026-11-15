use bitflags::bitflags;

bitflags! {
    #[derive(Default)]
    pub struct Flag : u8 {
        const ZERO          = 0b1000_0000;
        const NEGATIVE      = 0b0100_0000;
        const HALF_CARRY    = 0b0010_0000;
        const FULL_CARRY    = 0b0001_0000;
    }
}

impl crate::state::Stateful for Flag {
    fn save(&self, w: &mut crate::state::Writer) {
        self.bits().save(w);
    }
    fn load(&mut self, r: &mut crate::state::Reader) -> crate::state::StateResult {
        let mut bits = 0u8;
        bits.load(r)?;
        *self = Flag::from_bits_truncate(bits);
        Ok(())
    }
}
