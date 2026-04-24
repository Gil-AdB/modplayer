import re

def check_orphaned(filename):
    with open(filename, 'r') as f:
        content = f.read()
    ticks = content.split("[Order")
    
    for i, tick_str in enumerate(ticks[1:]):
        voices_header = re.search(r'Voices: (\d+)', tick_str)
        total_voices = int(voices_header.group(1)) if voices_header else 0
        
        # Split into Voices section and Channels section
        parts = tick_str.split("Channels:")
        if len(parts) < 2: continue
        
        voices_section = parts[0]
        channels_section = parts[1]
        
        voice_lines = re.findall(r'  Ch (\d+): ON', voices_section)
        channel_lines = re.findall(r'  Ch (\d+): ON', channels_section)
        
        if total_voices != len(voice_lines):
            print(f"Tick {i}: Header Voices count {total_voices} != Voice list count {len(voice_lines)}")
            
        if len(voice_lines) != len(channel_lines):
             # Extract tick info
            tick_info = re.search(r' \d+ \| Row \d+ \| Tick (\d+)\]', tick_str)
            tick_num = tick_info.group(1) if tick_info else "?"
            print(f"Tick {tick_num} (Index {i}): Active voices ({len(voice_lines)}) != Active channels ({len(channel_lines)})")

if __name__ == "__main__":
    import sys
    check_orphaned(sys.argv[1])
