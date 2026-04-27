import re

def check_orphans(filename):
    with open(filename, 'r') as f:
        content = f.read()
    ticks = content.split("[Order")
    
    for i, tick_str in enumerate(ticks[1:]):
        # All voices that are ON
        voices_on = re.findall(r'  V (\d+) \(Ch (\d+)\): ON', tick_str)
        # All channels and their assigned voice
        channels_raw = re.findall(r'  Ch (\d+): (ON|OFF) \| Voice: (.*)', tick_str)
        
        assigned_voices = set()
        for _, _, voice_str in channels_raw:
            if "Some(" in voice_str:
                v_idx = int(re.search(r'Some\((\d+)\)', voice_str).group(1))
                assigned_voices.add(v_idx)
        
        orphans = []
        for v_idx_str, ch_idx_str in voices_on:
            v_idx = int(v_idx_str)
            if v_idx not in assigned_voices:
                orphans.append(v_idx)
        
        if orphans:
            print(f"Tick index {i}: Voices ON but NOT assigned to any channel: {orphans}")

if __name__ == "__main__":
    import sys
    check_orphans(sys.argv[1])
