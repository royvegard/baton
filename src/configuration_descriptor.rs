struct Configuration {
    data: Vec<u8>,
}

impl Configuration {
    fn parse(&mut self) {
        let mut i = 0;
        while i < self.data.len() {
            let b_length = self.data[i];
            let b_descriptor_type = self.data[i + 1];

            match b_descriptor_type {
                // CONFIGURATION
                0x02 => self.parse_configuration(&self.data[i..i + b_length as usize]),
                // Class-Specific Audio Control Interface Descriptor
                0x24 => {
                    self.parse_class_specific_ac_descriptor(&self.data[i..i + b_length as usize])
                }
                _ => (),
            };

            i += b_length as usize;
        }
    }

    fn parse_configuration(&self, d: &[u8]) {
        let b_length: u8 = d[0];
        let b_descriptor_type: u8 = d[1];
        let w_total_length: u16 = ((d[3] as u16) << 8) | d[2] as u16;
        assert_eq!(b_length as usize, d.len());
        assert_eq!(b_descriptor_type, 0x02);
        assert_eq!(w_total_length as usize, self.data.len());

        let b_num_interfaces: u8 = d[4];
        let b_configuration_value: u8 = d[5];
        let i_configuration: u8 = d[6];
        let bm_attributes: u8 = d[7];
        let b_max_power: u8 = d[8];
    }

    fn parse_class_specific_ac_descriptor(&self, d: &[u8]) {
        const HEADER: u8 = 0x01;

        let b_length: u8 = d[0];
        let b_descriptor_type: u8 = d[1];
        assert_eq!(b_length as usize, d.len());
        assert_eq!(b_descriptor_type, 0x24);

        let b_descriptor_sub_type: u8 = d[2];
        println!("sub type: {}", b_descriptor_sub_type);
        match b_descriptor_sub_type {
            HEADER => parse_header_subtype(&d),
            _ => (),
        };

        fn parse_header_subtype(d: &[u8]) {
            let b_length: u8 = d[0];
            let b_descriptor_type: u8 = d[1];
            let b_descriptor_sub_type: u8 = d[2];
            assert_eq!(b_length as usize, d.len());
            assert_eq!(b_descriptor_type, 0x24);
            assert_eq!(b_descriptor_sub_type, HEADER);
        }
    }
}
