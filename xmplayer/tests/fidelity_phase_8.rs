#[cfg(test)]
mod tests {
    use xmplayer::test_utils::{MockSongBuilder};
    use xmplayer::module_reader::{SongType};
    use xmplayer::pattern::Pattern;
    use xmplayer::song::FilterType;
    use xmplayer::instrument::LoopType;

    struct MockBuffer {
        data: Vec<f32>,
    }

    impl xmplayer::song::BufferAdapter for MockBuffer {
        fn mix_sample(&mut self, _channel: usize, value: f32, pos: usize) {
            if pos >= self.data.len() {
                self.data.resize(pos + 1, 0.0);
            }
            self.data[pos] += value;
        }
        fn mix_samples(&mut self, _channel: usize, samples: &[f32], pos: usize) {
            if pos + samples.len() > self.data.len() {
                self.data.resize(pos + samples.len(), 0.0);
            }
            for (i, &s) in samples.iter().enumerate() {
                self.data[pos + i] += s;
            }
        }
        fn clear(&mut self) {}
        fn len(&mut self) -> usize { self.data.len() }
        fn num_frames(&mut self) -> usize { self.data.len() }
        fn post_process(&mut self) {}
    }

    #[test]
    fn test_linear_interpolation() {
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(64);
        
        let mut data = vec![0.0f32; 16];
        for i in 0..16 { data[i] = i as f32 / 16.0; }
        builder.instruments[1].samples[0].data = data;
        builder.instruments[1].samples[0].length = 16;
        builder.instruments[1].samples[0].loop_end = 16;
        builder.instruments[1].samples[0].setup_loops_and_padding();
        
        let mut tester = builder.get_tester();
        tester.song.filter = FilterType::Linear;
        tester.song.song_data.patterns[0].rows[0].channels[0] = Pattern {
            note: 48, instrument: 1, volume: 255, effect: 0, effect_param: 0,
        };
        
        tester.tick();
        
        let v_idx = tester.song.voices.iter().position(|v| v.on).expect("No active voice found");
        
        let voice = &mut tester.song.voices[v_idx];
        voice.sample_position = 8.5; 
        voice.du = 0.0; 
        voice.volume.output_volume = 1.0;
        tester.song.is_fast_forwarding = false;
        tester.song.master_volume = 128;
        tester.song.mixing_volume = 128;
        
        let mut mock_buf = MockBuffer { data: vec![0.0; 10] };
        tester.song.output_channels(0, &mut mock_buf, 1);
        
        assert!((mock_buf.data[0] - 0.19887).abs() < 0.001);
    }

    #[test]
    fn test_cubic_interpolation() {
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(64);
        
        let mut data = vec![0.0f32; 16];
        for i in 0..16 { data[i] = i as f32 / 16.0; }
        builder.instruments[1].samples[0].data = data;
        builder.instruments[1].samples[0].length = 16;
        builder.instruments[1].samples[0].loop_end = 16;
        builder.instruments[1].samples[0].setup_loops_and_padding();
        
        let mut tester = builder.get_tester();
        tester.song.filter = FilterType::Cubic;
        tester.song.song_data.patterns[0].rows[0].channels[0] = Pattern {
            note: 48, instrument: 1, volume: 255, effect: 0, effect_param: 0,
        };
        
        tester.tick();
        
        let v_idx = tester.song.voices.iter().position(|v| v.on).expect("No active voice found");
        
        let voice = &mut tester.song.voices[v_idx];
        voice.sample_position = 8.5; 
        voice.du = 0.0; 
        voice.volume.output_volume = 1.0;
        tester.song.is_fast_forwarding = false;
        tester.song.master_volume = 128;
        tester.song.mixing_volume = 128;
        
        let mut mock_buf = MockBuffer { data: vec![0.0; 10] };
        tester.song.output_channels(0, &mut mock_buf, 1);
        
        // Cubic with data = [..., 3/16, 4/16, 5/16, 6/16, ...] at x=0.5
        // Should be very close to Linear for linear data.
        println!("Cubic result: {}", mock_buf.data[0]);
        assert!((mock_buf.data[0] - 0.19887).abs() < 0.001);
    }

    #[test]
    fn test_sinc_interpolation() {
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(64);
        
        let mut data = vec![0.0f32; 16];
        for i in 0..16 { data[i] = i as f32 / 16.0; }
        builder.instruments[1].samples[0].data = data;
        builder.instruments[1].samples[0].length = 16;
        builder.instruments[1].samples[0].loop_end = 16;
        builder.instruments[1].samples[0].setup_loops_and_padding();
        
        let mut tester = builder.get_tester();
        tester.song.filter = FilterType::Sinc;
        tester.song.song_data.patterns[0].rows[0].channels[0] = Pattern {
            note: 48, instrument: 1, volume: 255, effect: 0, effect_param: 0,
        };
        
        tester.tick();
        
        let v_idx = tester.song.voices.iter().position(|v| v.on).expect("No active voice found");
        
        let voice = &mut tester.song.voices[v_idx];
        voice.sample_position = 8.5; 
        voice.du = 0.0; 
        voice.volume.output_volume = 1.0;
        tester.song.is_fast_forwarding = false;
        tester.song.master_volume = 128;
        tester.song.mixing_volume = 128;
        
        let mut mock_buf = MockBuffer { data: vec![0.0; 10] };
        tester.song.output_channels(0, &mut mock_buf, 1);
        
        println!("Sinc result: {}", mock_buf.data[0]);
        // Sinc for linear data should also be close.
        assert!((mock_buf.data[0] - 0.19887).abs() < 0.001);
    }

    #[test]
    fn test_loop_boundary_interpolation() {
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(64);
        
        let mut data = vec![0.0f32; 16];
        for i in 0..16 { data[i] = i as f32 / 16.0; }
        builder.instruments[1].samples[0].data = data;
        builder.instruments[1].samples[0].length = 16;
        builder.instruments[1].samples[0].loop_start = 0;
        builder.instruments[1].samples[0].loop_end = 16;
        builder.instruments[1].samples[0].loop_len = 16;
        builder.instruments[1].samples[0].loop_type = LoopType::ForwardLoop;
        builder.instruments[1].samples[0].setup_loops_and_padding();
        
        let mut tester = builder.get_tester();
        tester.song.filter = FilterType::Linear;
        tester.song.song_data.patterns[0].rows[0].channels[0] = Pattern {
            note: 48, instrument: 1, volume: 255, effect: 0, effect_param: 0,
        };
        
        tester.tick();
        
        let v_idx = tester.song.voices.iter().position(|v| v.on).expect("No active voice found");

        let voice = &mut tester.song.voices[v_idx];
        voice.sample_position = 19.5;
        voice.du = 0.0;
        voice.volume.output_volume = 1.0;
        tester.song.is_fast_forwarding = false;
        tester.song.master_volume = 128;
        tester.song.mixing_volume = 128;
        
        let mut mock_buf = MockBuffer { data: vec![0.0; 10] };
        tester.song.output_channels(0, &mut mock_buf, 1);
        
        assert!((mock_buf.data[0] - 0.33145).abs() < 0.001);
    }
}
