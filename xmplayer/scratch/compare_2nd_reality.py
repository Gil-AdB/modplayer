import sys

def parse_dump(filename):
    ticks = []
    current_tick = None
    with open(filename, 'r') as f:
        for line in f:
            if line.startswith('[Order'):
                if current_tick:
                    ticks.append(current_tick)
                # [Order 000 | Row 000 | Tick 000] (Voices: 0 / Channels: 0) (Speed: 6 / BPM: 125 / GVol: 64)
                parts = line.split(']')
                header = parts[0].replace('[', '').split('|')
                order = int(header[0].split()[1])
                row = int(header[1].split()[1])
                tick = int(header[2].split()[1])
                current_tick = {'order': order, 'row': row, 'tick': tick, 'voices': {}}
            elif line.startswith('  Ch '):
                #   Ch 00: ON | Inst 02 | Samp 00 | Pos     4.000 | dU   0.180 | Vol   0.016 | Pan 102 (102) | Sus Y | Env V:000 P:000 | Eff 04 10
                parts = line.split('|')
                ch_id = int(parts[0].split()[1].replace(':', ''))
                inst = int(parts[1].split()[1])
                vol = float(parts[5].split()[1])
                eff = parts[9].split()[1]
                param = parts[9].split()[2]
                current_tick['voices'][ch_id] = {'inst': inst, 'vol': vol, 'eff': eff, 'param': param}
    if current_tick:
        ticks.append(current_tick)
    return ticks

def compare(master_ticks, refactor_ticks):
    for i in range(min(len(master_ticks), len(refactor_ticks))):
        m = master_ticks[i]
        r = refactor_ticks[i]
        
        if m['order'] != r['order'] or m['row'] != r['row'] or m['tick'] != r['tick']:
            print(f"Tick mismatch at index {i}: Master={m['order']}:{m['row']}:{m['tick']} vs Refactor={r['order']}:{r['row']}:{r['tick']}")
            break
            
        # Check Channel 8 specifically
        ch = 8
        if ch in m['voices'] or ch in r['voices']:
            mv = m['voices'].get(ch)
            rv = r['voices'].get(ch)
            
            if mv != rv:
                print(f"[O:{m['order']:03} R:{m['row']:03} T:{m['tick']:03}] Channel {ch:02} mismatch:")
                print(f"  Master:   {mv}")
                print(f"  Refactor: {rv}")

if __name__ == "__main__":
    master = parse_dump('xmplayer/test_data/2nd_reality_master.txt')
    refactor = parse_dump('xmplayer/test_data/2nd_reality_refactor.txt')
    compare(master, refactor)
