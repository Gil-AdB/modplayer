import re

def check_duplicates(filename):
    with open(filename, 'r') as f:
        content = f.read()
    ticks = content.split("[Order")
    
    for i, tick_str in enumerate(ticks[1:]):
        channels_raw = re.findall(r'  Ch (\d+): (ON|OFF) \| Voice: (.*)', tick_str)
        
        assigned_voices = {}
        for ch_idx_str, _, voice_str in channels_raw:
            if "Some(" in voice_str:
                v_idx = int(re.search(r'Some\((\d+)\)', voice_str).group(1))
                if v_idx in assigned_voices:
                    print(f"Tick index {i}: Voice {v_idx} assigned to multiple channels: {assigned_voices[v_idx]} and {ch_idx_str}")
                assigned_voices[v_idx] = ch_idx_str

if __name__ == "__main__":
    import sys
    check_duplicates(sys.argv[1])
