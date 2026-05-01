import sys

filename = sys.argv[1] if len(sys.argv) > 1 else '/Users/gil-ad/Downloads/mods/2nd_reality.s3m'
with open(filename, 'rb') as f:
    f.seek(0x40) # Channel data starts at 0x40
    channel_data = f.read(32)
    print("Channel Data:", list(channel_data))
    num_channels = 0
    for i, b in enumerate(channel_data):
        if b != 255:
            num_channels += 1
            print(f"Channel {i}: {b}")
    print("Total channels:", num_channels)
