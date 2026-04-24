import re

def parse_dump(filename):
    with open(filename, 'r') as f:
        content = f.read()
    ticks = content.split("[Order")
    data = {}
    for tick_str in ticks[1:]:
        header = re.search(r' (\d+) \| Row (\d+) \| Tick (\d+)\]', tick_str)
        if not header: continue
        key = tuple(map(int, header.groups()))
        
        # Split into Voices section and Channels section
        parts = tick_str.split("Channels:")
        voices_section = parts[0]
        
        # Try both formats: "Ch 00: ON" (master) and "V 000 (Ch 00): ON" (refactor)
        voice_lines_old = re.findall(r'  Ch (\d+): ON', voices_section)
        voice_lines_new = re.findall(r'  V \d+ \(Ch \d+\): ON', voices_section)
        
        count = len(voice_lines_old) if voice_lines_old else len(voice_lines_new)
        data[key] = count
    return data

def compare(master_file, refactor_file):
    m_data = parse_dump(master_file)
    r_data = parse_dump(refactor_file)
    
    all_keys = sorted(set(m_data.keys()) | set(r_data.keys()))
    
    print(f"{'Order/Row/Tick':<20} | {'Master':<10} | {'Refactor':<10} | {'Diff':<10}")
    print("-" * 55)
    for key in all_keys:
        m_count = m_data.get(key, 0)
        r_count = r_data.get(key, 0)
        if m_count != r_count:
            print(f"{str(key):<20} | {m_count:<10} | {r_count:<10} | {r_count - m_count:<10}")

if __name__ == "__main__":
    import sys
    compare("test_data/strshine_master_short.txt", "test_data/strshine_refactor.txt")
