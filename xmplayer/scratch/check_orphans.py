import re

def check_orphans(filename):
    with open(filename, 'r') as f:
        content = f.read()
    
    ticks = re.split(r'\[Order \d+ \| Row \d+ \| Tick \d+\]', content)
    
    for i, tick_data in enumerate(ticks):
        if not tick_data.strip(): continue
        
        voices = re.findall(r'V (\d+) \(Ch (\d+)\): ON', tick_data)
        # Ch 00: ON | Voice: Some(0)
        channel_voice_map = {}
        ch_lines = re.findall(r'Ch (\d+): ON \| Voice: Some\((\d+)\)', tick_data)
        for ch, v in ch_lines:
            channel_voice_map[int(ch)] = int(v)
        
        for v_idx, ch_idx in voices:
            v_idx = int(v_idx)
            ch_idx = int(ch_idx)
            if ch_idx not in channel_voice_map or channel_voice_map[ch_idx] != v_idx:
                print(f"Tick {i}: Orphan Voice {v_idx} on Channel {ch_idx} (Channel thinks voice is {channel_voice_map.get(ch_idx)})")

if __name__ == "__main__":
    check_orphans('test_data/strshine_refactor.txt')
