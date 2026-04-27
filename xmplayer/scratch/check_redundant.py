import re
from collections import Counter

def check_redundant_voices(filename):
    with open(filename, 'r') as f:
        content = f.read()
        
    ticks = content.split("[Order")
    
    for i, tick_str in enumerate(ticks[1:]): # skip first split part
        channels = re.findall(r'Ch (\d+): ON', tick_str)
        counts = Counter(channels)
        duplicates = {ch: count for ch, count in counts.items() if count > 1}
        
        if duplicates:
            # Extract tick info
            tick_info = re.search(r' \d+ \| Row \d+ \| Tick (\d+)\]', tick_str)
            tick_num = tick_info.group(1) if tick_info else "?"
            print(f"Tick {tick_num} (Absolute Index {i}): Redundant voices for channels {duplicates}")

if __name__ == "__main__":
    import sys
    check_redundant_voices(sys.argv[1])
