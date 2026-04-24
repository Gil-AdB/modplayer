import re

def check_leaks(filename):
    with open(filename, 'r') as f:
        content = f.read()
    
    ticks = re.split(r'\[Order \d+ \| Row \d+ \| Tick \d+\]', content)
    
    for i, tick_data in enumerate(ticks):
        if not tick_data.strip(): continue
        
        voices = re.findall(r'V (\d+) \(Ch (\d+)\): ON', tick_data)
        channels = re.findall(r'Ch (\d+): ON', tick_data)
        
        voice_channels = [v[1] for v in voices]
        
        # Check for multiple voices on the same channel
        seen = set()
        dupes = set()
        for c in voice_channels:
            if c in seen:
                dupes.add(c)
            seen.add(c)
        
        if dupes:
            print(f"Tick {i}: Multiple voices on channels {dupes}")
            # print(tick_data)
            # break

if __name__ == "__main__":
    check_leaks('test_data/strshine_refactor.txt')
