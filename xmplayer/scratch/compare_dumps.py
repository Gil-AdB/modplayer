import sys
import re

def normalize(line):
    # Remove metadata in brackets
    line = re.sub(r'\(Voices: \d+ / Channels: \d+\) \(Speed: \d+ / BPM: \d+\)', '', line)
    # Remove effects
    line = re.sub(r'\| Eff [0-9a-f ]+', '', line)
    # Normalize Sus
    line = line.replace('Sus N', 'Sus Y') # Ignore Sus diff for now
    # Normalize Pan (master has 128 (128))
    line = re.sub(r'Pan \d+ \(\d+\)', 'Pan 128 (128)', line)
    return line.strip()

def compare(file1, file2):
    with open(file1) as f1, open(file2) as f2:
        l1 = f1.readlines()
        l2 = f2.readlines()
    
    i = 0
    j = 0
    while i < len(l1) and j < len(l2):
        n1 = normalize(l1[i])
        n2 = normalize(l2[j])
        
        if not n1:
            i += 1
            continue
        if not n2:
            j += 1
            continue
            
        if n1 != n2:
            print(f"Difference at file1 line {i+1}, file2 line {j+1}")
            print(f"Master: {l1[i].strip()}")
            print(f"Refac : {l2[j].strip()}")
            return
        
        i += 1
        j += 1

compare(sys.argv[1], sys.argv[2])
