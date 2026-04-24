from xmplayer.module_reader import read_module
import sys

def dump_order_10(filename):
    song_data = read_module(filename)
    order_10_idx = song_data.pattern_order[10]
    pattern = song_data.patterns[order_10_idx]
    
    print(f"Order 10 -> Pattern {order_10_idx}")
    for r_idx, row in enumerate(pattern.rows):
        has_data = False
        for ch in row.channels:
            if ch.note != 0 or ch.instrument != 0 or ch.volume != 255 or ch.effect != 0:
                has_data = True
                break
        if not has_data: continue
        
        print(f"Row {r_idx:02}: ", end="")
        for c_idx, ch in enumerate(row.channels):
            if ch.note != 0 or ch.instrument != 0 or ch.volume != 255 or ch.effect != 0:
                print(f"| Ch{c_idx:02} N:{ch.note:3} I:{ch.instrument:2} V:{ch.volume:3} E:{ch.effect:02X} P:{ch.effect_param:02X} ", end="")
        print("|")

if __name__ == "__main__":
    dump_order_10(sys.argv[1])
