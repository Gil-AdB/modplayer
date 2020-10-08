pub const ARP_TAB: [u8; 100] =
[
0,1,2,0,1,2,0,1,2,0,1,2,0,1,2,0,

/* The following are overflown bytes from FT2's binary.
** (confirmed to be the same on FT2.08 and FT2.09)
*/
0x00, 0x18, 0x31, 0x4A, 0x61, 0x78, 0x8D, 0xA1, 0xB4, 0xC5, 0xD4,
0xE0, 0xEB, 0xF4, 0xFA, 0xFD, 0xFF, 0xFD, 0xFA, 0xF4, 0xEB, 0xE0,
0xD4, 0xC5, 0xB4, 0xA1, 0x8D, 0x78, 0x61, 0x4A, 0x31, 0x18, 0x01,
0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x03, 0x00, 0x02,
0x00, 0x04, 0x00, 0x00, 0x00, 0x05, 0x06, 0x00, 0x00, 0x07, 0x00,
0x01, 0x00, 0x02, 0x00, 0x03, 0x04, 0x05, 0x00, 0x00, 0x0B, 0x00,
0x0A, 0x02, 0x01, 0x03, 0x04, 0x07, 0x00, 0x05, 0x06, 0x00, 0x00,
0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00
];

pub const VIB_SINE_TAB: [i8;256] = // for auto-vibrato
[
0,  -2,  -3,  -5,  -6,  -8,  -9, -11, -12, -14, -16, -17, -19, -20, -22, -23,
-24, -26, -27, -29, -30, -32, -33, -34, -36, -37, -38, -39, -41, -42, -43, -44,
-45, -46, -47, -48, -49, -50, -51, -52, -53, -54, -55, -56, -56, -57, -58, -59,
-59, -60, -60, -61, -61, -62, -62, -62, -63, -63, -63, -64, -64, -64, -64, -64,
-64, -64, -64, -64, -64, -64, -63, -63, -63, -62, -62, -62, -61, -61, -60, -60,
-59, -59, -58, -57, -56, -56, -55, -54, -53, -52, -51, -50, -49, -48, -47, -46,
-45, -44, -43, -42, -41, -39, -38, -37, -36, -34, -33, -32, -30, -29, -27, -26,
-24, -23, -22, -20, -19, -17, -16, -14, -12, -11,  -9,  -8,  -6,  -5,  -3,  -2,
0,   2,   3,   5,   6,   8,   9,  11,  12,  14,  16,  17,  19,  20,  22,  23,
24,  26,  27,  29,  30,  32,  33,  34,  36,  37,  38,  39,  41,  42,  43,  44,
45,  46,  47,  48,  49,  50,  51,  52,  53,  54,  55,  56,  56,  57,  58,  59,
59,  60,  60,  61,  61,  62,  62,  62,  63,  63,  63,  64,  64,  64,  64,  64,
64,  64,  64,  64,  64,  64,  63,  63,  63,  62,  62,  62,  61,  61,  60,  60,
59,  59,  58,  57,  56,  56,  55,  54,  53,  52,  51,  50,  49,  48,  47,  46,
45,  44,  43,  42,  41,  39,  38,  37,  36,  34,  33,  32,  30,  29,  27,  26,
24,  23,  22,  20,  19,  17,  16,  14,  12,  11,   9,   8,   6,   5,   3,   2
];

pub const VIB_TAB: [u8;32] = // for normal vibrato/tremolo
[
0, 24, 49, 74, 97,120,141,161,180,197,212,224,235,244,250,253,
255,253,250,244,235,224,212,197,180,161,141,120, 97, 74, 49, 24
];

pub const AMIGA_PERIOD:[u16; 12 * 8] = // used for .MOD loading/saving
[
6848, 6464, 6096, 5760, 5424, 5120, 4832, 4560, 4304, 4064, 3840, 3624,
3424, 3232, 3048, 2880, 2712, 2560, 2416, 2280, 2152, 2032, 1920, 1812,
1712, 1616, 1524, 1440, 1356, 1280, 1208, 1140, 1076, 1016,  960,  906,
856,  808,  762,  720,  678,  640,  604,  570,  538,  508,  480,  453,
428,  404,  381,  360,  339,  320,  302,  285,  269,  254,  240,  226,
214,  202,  190,  180,  170,  160,  151,  143,  135,  127,  120,  113,
107,  101,   95,   90,   85,   80,   75,   71,   67,   63,   60,   56,
53,   50,   47,   45,   42,   40,   37,   35,   33,   31,   30,   28
];

pub const LINEAR_PERIODS: [u16;1936] = // bit-exact to FT2 table
[
7744, 7740, 7736, 7732, 7728, 7724, 7720, 7716, 7712, 7708, 7704, 7700, 7696, 7692, 7688, 7684,
7680, 7676, 7672, 7668, 7664, 7660, 7656, 7652, 7648, 7644, 7640, 7636, 7632, 7628, 7624, 7620,
7616, 7612, 7608, 7604, 7600, 7596, 7592, 7588, 7584, 7580, 7576, 7572, 7568, 7564, 7560, 7556,
7552, 7548, 7544, 7540, 7536, 7532, 7528, 7524, 7520, 7516, 7512, 7508, 7504, 7500, 7496, 7492,
7488, 7484, 7480, 7476, 7472, 7468, 7464, 7460, 7456, 7452, 7448, 7444, 7440, 7436, 7432, 7428,
7424, 7420, 7416, 7412, 7408, 7404, 7400, 7396, 7392, 7388, 7384, 7380, 7376, 7372, 7368, 7364,
7360, 7356, 7352, 7348, 7344, 7340, 7336, 7332, 7328, 7324, 7320, 7316, 7312, 7308, 7304, 7300,
7296, 7292, 7288, 7284, 7280, 7276, 7272, 7268, 7264, 7260, 7256, 7252, 7248, 7244, 7240, 7236,
7232, 7228, 7224, 7220, 7216, 7212, 7208, 7204, 7200, 7196, 7192, 7188, 7184, 7180, 7176, 7172,
7168, 7164, 7160, 7156, 7152, 7148, 7144, 7140, 7136, 7132, 7128, 7124, 7120, 7116, 7112, 7108,
7104, 7100, 7096, 7092, 7088, 7084, 7080, 7076, 7072, 7068, 7064, 7060, 7056, 7052, 7048, 7044,
7040, 7036, 7032, 7028, 7024, 7020, 7016, 7012, 7008, 7004, 7000, 6996, 6992, 6988, 6984, 6980,
6976, 6972, 6968, 6964, 6960, 6956, 6952, 6948, 6944, 6940, 6936, 6932, 6928, 6924, 6920, 6916,
6912, 6908, 6904, 6900, 6896, 6892, 6888, 6884, 6880, 6876, 6872, 6868, 6864, 6860, 6856, 6852,
6848, 6844, 6840, 6836, 6832, 6828, 6824, 6820, 6816, 6812, 6808, 6804, 6800, 6796, 6792, 6788,
6784, 6780, 6776, 6772, 6768, 6764, 6760, 6756, 6752, 6748, 6744, 6740, 6736, 6732, 6728, 6724,
6720, 6716, 6712, 6708, 6704, 6700, 6696, 6692, 6688, 6684, 6680, 6676, 6672, 6668, 6664, 6660,
6656, 6652, 6648, 6644, 6640, 6636, 6632, 6628, 6624, 6620, 6616, 6612, 6608, 6604, 6600, 6596,
6592, 6588, 6584, 6580, 6576, 6572, 6568, 6564, 6560, 6556, 6552, 6548, 6544, 6540, 6536, 6532,
6528, 6524, 6520, 6516, 6512, 6508, 6504, 6500, 6496, 6492, 6488, 6484, 6480, 6476, 6472, 6468,
6464, 6460, 6456, 6452, 6448, 6444, 6440, 6436, 6432, 6428, 6424, 6420, 6416, 6412, 6408, 6404,
6400, 6396, 6392, 6388, 6384, 6380, 6376, 6372, 6368, 6364, 6360, 6356, 6352, 6348, 6344, 6340,
6336, 6332, 6328, 6324, 6320, 6316, 6312, 6308, 6304, 6300, 6296, 6292, 6288, 6284, 6280, 6276,
6272, 6268, 6264, 6260, 6256, 6252, 6248, 6244, 6240, 6236, 6232, 6228, 6224, 6220, 6216, 6212,
6208, 6204, 6200, 6196, 6192, 6188, 6184, 6180, 6176, 6172, 6168, 6164, 6160, 6156, 6152, 6148,
6144, 6140, 6136, 6132, 6128, 6124, 6120, 6116, 6112, 6108, 6104, 6100, 6096, 6092, 6088, 6084,
6080, 6076, 6072, 6068, 6064, 6060, 6056, 6052, 6048, 6044, 6040, 6036, 6032, 6028, 6024, 6020,
6016, 6012, 6008, 6004, 6000, 5996, 5992, 5988, 5984, 5980, 5976, 5972, 5968, 5964, 5960, 5956,
5952, 5948, 5944, 5940, 5936, 5932, 5928, 5924, 5920, 5916, 5912, 5908, 5904, 5900, 5896, 5892,
5888, 5884, 5880, 5876, 5872, 5868, 5864, 5860, 5856, 5852, 5848, 5844, 5840, 5836, 5832, 5828,
5824, 5820, 5816, 5812, 5808, 5804, 5800, 5796, 5792, 5788, 5784, 5780, 5776, 5772, 5768, 5764,
5760, 5756, 5752, 5748, 5744, 5740, 5736, 5732, 5728, 5724, 5720, 5716, 5712, 5708, 5704, 5700,
5696, 5692, 5688, 5684, 5680, 5676, 5672, 5668, 5664, 5660, 5656, 5652, 5648, 5644, 5640, 5636,
5632, 5628, 5624, 5620, 5616, 5612, 5608, 5604, 5600, 5596, 5592, 5588, 5584, 5580, 5576, 5572,
5568, 5564, 5560, 5556, 5552, 5548, 5544, 5540, 5536, 5532, 5528, 5524, 5520, 5516, 5512, 5508,
5504, 5500, 5496, 5492, 5488, 5484, 5480, 5476, 5472, 5468, 5464, 5460, 5456, 5452, 5448, 5444,
5440, 5436, 5432, 5428, 5424, 5420, 5416, 5412, 5408, 5404, 5400, 5396, 5392, 5388, 5384, 5380,
5376, 5372, 5368, 5364, 5360, 5356, 5352, 5348, 5344, 5340, 5336, 5332, 5328, 5324, 5320, 5316,
5312, 5308, 5304, 5300, 5296, 5292, 5288, 5284, 5280, 5276, 5272, 5268, 5264, 5260, 5256, 5252,
5248, 5244, 5240, 5236, 5232, 5228, 5224, 5220, 5216, 5212, 5208, 5204, 5200, 5196, 5192, 5188,
5184, 5180, 5176, 5172, 5168, 5164, 5160, 5156, 5152, 5148, 5144, 5140, 5136, 5132, 5128, 5124,
5120, 5116, 5112, 5108, 5104, 5100, 5096, 5092, 5088, 5084, 5080, 5076, 5072, 5068, 5064, 5060,
5056, 5052, 5048, 5044, 5040, 5036, 5032, 5028, 5024, 5020, 5016, 5012, 5008, 5004, 5000, 4996,
4992, 4988, 4984, 4980, 4976, 4972, 4968, 4964, 4960, 4956, 4952, 4948, 4944, 4940, 4936, 4932,
4928, 4924, 4920, 4916, 4912, 4908, 4904, 4900, 4896, 4892, 4888, 4884, 4880, 4876, 4872, 4868,
4864, 4860, 4856, 4852, 4848, 4844, 4840, 4836, 4832, 4828, 4824, 4820, 4816, 4812, 4808, 4804,
4800, 4796, 4792, 4788, 4784, 4780, 4776, 4772, 4768, 4764, 4760, 4756, 4752, 4748, 4744, 4740,
4736, 4732, 4728, 4724, 4720, 4716, 4712, 4708, 4704, 4700, 4696, 4692, 4688, 4684, 4680, 4676,
4672, 4668, 4664, 4660, 4656, 4652, 4648, 4644, 4640, 4636, 4632, 4628, 4624, 4620, 4616, 4612,
4608, 4604, 4600, 4596, 4592, 4588, 4584, 4580, 4576, 4572, 4568, 4564, 4560, 4556, 4552, 4548,
4544, 4540, 4536, 4532, 4528, 4524, 4520, 4516, 4512, 4508, 4504, 4500, 4496, 4492, 4488, 4484,
4480, 4476, 4472, 4468, 4464, 4460, 4456, 4452, 4448, 4444, 4440, 4436, 4432, 4428, 4424, 4420,
4416, 4412, 4408, 4404, 4400, 4396, 4392, 4388, 4384, 4380, 4376, 4372, 4368, 4364, 4360, 4356,
4352, 4348, 4344, 4340, 4336, 4332, 4328, 4324, 4320, 4316, 4312, 4308, 4304, 4300, 4296, 4292,
4288, 4284, 4280, 4276, 4272, 4268, 4264, 4260, 4256, 4252, 4248, 4244, 4240, 4236, 4232, 4228,
4224, 4220, 4216, 4212, 4208, 4204, 4200, 4196, 4192, 4188, 4184, 4180, 4176, 4172, 4168, 4164,
4160, 4156, 4152, 4148, 4144, 4140, 4136, 4132, 4128, 4124, 4120, 4116, 4112, 4108, 4104, 4100,
4096, 4092, 4088, 4084, 4080, 4076, 4072, 4068, 4064, 4060, 4056, 4052, 4048, 4044, 4040, 4036,
4032, 4028, 4024, 4020, 4016, 4012, 4008, 4004, 4000, 3996, 3992, 3988, 3984, 3980, 3976, 3972,
3968, 3964, 3960, 3956, 3952, 3948, 3944, 3940, 3936, 3932, 3928, 3924, 3920, 3916, 3912, 3908,
3904, 3900, 3896, 3892, 3888, 3884, 3880, 3876, 3872, 3868, 3864, 3860, 3856, 3852, 3848, 3844,
3840, 3836, 3832, 3828, 3824, 3820, 3816, 3812, 3808, 3804, 3800, 3796, 3792, 3788, 3784, 3780,
3776, 3772, 3768, 3764, 3760, 3756, 3752, 3748, 3744, 3740, 3736, 3732, 3728, 3724, 3720, 3716,
3712, 3708, 3704, 3700, 3696, 3692, 3688, 3684, 3680, 3676, 3672, 3668, 3664, 3660, 3656, 3652,
3648, 3644, 3640, 3636, 3632, 3628, 3624, 3620, 3616, 3612, 3608, 3604, 3600, 3596, 3592, 3588,
3584, 3580, 3576, 3572, 3568, 3564, 3560, 3556, 3552, 3548, 3544, 3540, 3536, 3532, 3528, 3524,
3520, 3516, 3512, 3508, 3504, 3500, 3496, 3492, 3488, 3484, 3480, 3476, 3472, 3468, 3464, 3460,
3456, 3452, 3448, 3444, 3440, 3436, 3432, 3428, 3424, 3420, 3416, 3412, 3408, 3404, 3400, 3396,
3392, 3388, 3384, 3380, 3376, 3372, 3368, 3364, 3360, 3356, 3352, 3348, 3344, 3340, 3336, 3332,
3328, 3324, 3320, 3316, 3312, 3308, 3304, 3300, 3296, 3292, 3288, 3284, 3280, 3276, 3272, 3268,
3264, 3260, 3256, 3252, 3248, 3244, 3240, 3236, 3232, 3228, 3224, 3220, 3216, 3212, 3208, 3204,
3200, 3196, 3192, 3188, 3184, 3180, 3176, 3172, 3168, 3164, 3160, 3156, 3152, 3148, 3144, 3140,
3136, 3132, 3128, 3124, 3120, 3116, 3112, 3108, 3104, 3100, 3096, 3092, 3088, 3084, 3080, 3076,
3072, 3068, 3064, 3060, 3056, 3052, 3048, 3044, 3040, 3036, 3032, 3028, 3024, 3020, 3016, 3012,
3008, 3004, 3000, 2996, 2992, 2988, 2984, 2980, 2976, 2972, 2968, 2964, 2960, 2956, 2952, 2948,
2944, 2940, 2936, 2932, 2928, 2924, 2920, 2916, 2912, 2908, 2904, 2900, 2896, 2892, 2888, 2884,
2880, 2876, 2872, 2868, 2864, 2860, 2856, 2852, 2848, 2844, 2840, 2836, 2832, 2828, 2824, 2820,
2816, 2812, 2808, 2804, 2800, 2796, 2792, 2788, 2784, 2780, 2776, 2772, 2768, 2764, 2760, 2756,
2752, 2748, 2744, 2740, 2736, 2732, 2728, 2724, 2720, 2716, 2712, 2708, 2704, 2700, 2696, 2692,
2688, 2684, 2680, 2676, 2672, 2668, 2664, 2660, 2656, 2652, 2648, 2644, 2640, 2636, 2632, 2628,
2624, 2620, 2616, 2612, 2608, 2604, 2600, 2596, 2592, 2588, 2584, 2580, 2576, 2572, 2568, 2564,
2560, 2556, 2552, 2548, 2544, 2540, 2536, 2532, 2528, 2524, 2520, 2516, 2512, 2508, 2504, 2500,
2496, 2492, 2488, 2484, 2480, 2476, 2472, 2468, 2464, 2460, 2456, 2452, 2448, 2444, 2440, 2436,
2432, 2428, 2424, 2420, 2416, 2412, 2408, 2404, 2400, 2396, 2392, 2388, 2384, 2380, 2376, 2372,
2368, 2364, 2360, 2356, 2352, 2348, 2344, 2340, 2336, 2332, 2328, 2324, 2320, 2316, 2312, 2308,
2304, 2300, 2296, 2292, 2288, 2284, 2280, 2276, 2272, 2268, 2264, 2260, 2256, 2252, 2248, 2244,
2240, 2236, 2232, 2228, 2224, 2220, 2216, 2212, 2208, 2204, 2200, 2196, 2192, 2188, 2184, 2180,
2176, 2172, 2168, 2164, 2160, 2156, 2152, 2148, 2144, 2140, 2136, 2132, 2128, 2124, 2120, 2116,
2112, 2108, 2104, 2100, 2096, 2092, 2088, 2084, 2080, 2076, 2072, 2068, 2064, 2060, 2056, 2052,
2048, 2044, 2040, 2036, 2032, 2028, 2024, 2020, 2016, 2012, 2008, 2004, 2000, 1996, 1992, 1988,
1984, 1980, 1976, 1972, 1968, 1964, 1960, 1956, 1952, 1948, 1944, 1940, 1936, 1932, 1928, 1924,
1920, 1916, 1912, 1908, 1904, 1900, 1896, 1892, 1888, 1884, 1880, 1876, 1872, 1868, 1864, 1860,
1856, 1852, 1848, 1844, 1840, 1836, 1832, 1828, 1824, 1820, 1816, 1812, 1808, 1804, 1800, 1796,
1792, 1788, 1784, 1780, 1776, 1772, 1768, 1764, 1760, 1756, 1752, 1748, 1744, 1740, 1736, 1732,
1728, 1724, 1720, 1716, 1712, 1708, 1704, 1700, 1696, 1692, 1688, 1684, 1680, 1676, 1672, 1668,
1664, 1660, 1656, 1652, 1648, 1644, 1640, 1636, 1632, 1628, 1624, 1620, 1616, 1612, 1608, 1604,
1600, 1596, 1592, 1588, 1584, 1580, 1576, 1572, 1568, 1564, 1560, 1556, 1552, 1548, 1544, 1540,
1536, 1532, 1528, 1524, 1520, 1516, 1512, 1508, 1504, 1500, 1496, 1492, 1488, 1484, 1480, 1476,
1472, 1468, 1464, 1460, 1456, 1452, 1448, 1444, 1440, 1436, 1432, 1428, 1424, 1420, 1416, 1412,
1408, 1404, 1400, 1396, 1392, 1388, 1384, 1380, 1376, 1372, 1368, 1364, 1360, 1356, 1352, 1348,
1344, 1340, 1336, 1332, 1328, 1324, 1320, 1316, 1312, 1308, 1304, 1300, 1296, 1292, 1288, 1284,
1280, 1276, 1272, 1268, 1264, 1260, 1256, 1252, 1248, 1244, 1240, 1236, 1232, 1228, 1224, 1220,
1216, 1212, 1208, 1204, 1200, 1196, 1192, 1188, 1184, 1180, 1176, 1172, 1168, 1164, 1160, 1156,
1152, 1148, 1144, 1140, 1136, 1132, 1128, 1124, 1120, 1116, 1112, 1108, 1104, 1100, 1096, 1092,
1088, 1084, 1080, 1076, 1072, 1068, 1064, 1060, 1056, 1052, 1048, 1044, 1040, 1036, 1032, 1028,
1024, 1020, 1016, 1012, 1008, 1004, 1000,  996,  992,  988,  984,  980,  976,  972,  968,  964,
960,  956,  952,  948,  944,  940,  936,  932,  928,  924,  920,  916,  912,  908,  904,  900,
896,  892,  888,  884,  880,  876,  872,  868,  864,  860,  856,  852,  848,  844,  840,  836,
832,  828,  824,  820,  816,  812,  808,  804,  800,  796,  792,  788,  784,  780,  776,  772,
768,  764,  760,  756,  752,  748,  744,  740,  736,  732,  728,  724,  720,  716,  712,  708,
704,  700,  696,  692,  688,  684,  680,  676,  672,  668,  664,  660,  656,  652,  648,  644,
640,  636,  632,  628,  624,  620,  616,  612,  608,  604,  600,  596,  592,  588,  584,  580,
576,  572,  568,  564,  560,  556,  552,  548,  544,  540,  536,  532,  528,  524,  520,  516,
512,  508,  504,  500,  496,  492,  488,  484,  480,  476,  472,  468,  464,  460,  456,  452,
448,  444,  440,  436,  432,  428,  424,  420,  416,  412,  408,  404,  400,  396,  392,  388,
384,  380,  376,  372,  368,  364,  360,  356,  352,  348,  344,  340,  336,  332,  328,  324,
320,  316,  312,  308,  304,  300,  296,  292,  288,  284,  280,  276,  272,  268,  264,  260,
256,  252,  248,  244,  240,  236,  232,  228,  224,  220,  216,  212,  208,  204,  200,  196,
192,  188,  184,  180,  176,  172,  168,  164,  160,  156,  152,  148,  144,  140,  136,  132,
128,  124,  120,  116,  112,  108,  104,  100,   96,   92,   88,   84,   80,   76,   72,   68,
64,   60,   56,   52,   48,   44,   40,   36,   32,   28,   24,   20,   16,   12,    8,    4
];

pub const AMIGA_PERIODS:[u16;1936] = // bit-exact to FT2 table
[
29024, 28912, 28800, 28704, 28608, 28496, 28384, 28288, 28192, 28096, 28000, 27888, 27776, 27680, 27584, 27488,
27392, 27296, 27200, 27104, 27008, 26912, 26816, 26720, 26624, 26528, 26432, 26336, 26240, 26144, 26048, 25952,
25856, 25760, 25664, 25568, 25472, 25392, 25312, 25216, 25120, 25024, 24928, 24848, 24768, 24672, 24576, 24480,
24384, 24304, 24224, 24144, 24064, 23968, 23872, 23792, 23712, 23632, 23552, 23456, 23360, 23280, 23200, 23120,
23040, 22960, 22880, 22784, 22688, 22608, 22528, 22448, 22368, 22288, 22208, 22128, 22048, 21968, 21888, 21792,
21696, 21648, 21600, 21520, 21440, 21360, 21280, 21200, 21120, 21040, 20960, 20896, 20832, 20752, 20672, 20576,
20480, 20416, 20352, 20288, 20224, 20160, 20096, 20016, 19936, 19872, 19808, 19728, 19648, 19584, 19520, 19424,
19328, 19280, 19232, 19168, 19104, 19024, 18944, 18880, 18816, 18752, 18688, 18624, 18560, 18480, 18400, 18320,
18240, 18192, 18144, 18080, 18016, 17952, 17888, 17824, 17760, 17696, 17632, 17568, 17504, 17440, 17376, 17296,
17216, 17168, 17120, 17072, 17024, 16960, 16896, 16832, 16768, 16704, 16640, 16576, 16512, 16464, 16416, 16336,
16256, 16208, 16160, 16112, 16064, 16000, 15936, 15872, 15808, 15760, 15712, 15648, 15584, 15536, 15488, 15424,
15360, 15312, 15264, 15216, 15168, 15104, 15040, 14992, 14944, 14880, 14816, 14768, 14720, 14672, 14624, 14568,
14512, 14456, 14400, 14352, 14304, 14248, 14192, 14144, 14096, 14048, 14000, 13944, 13888, 13840, 13792, 13744,
13696, 13648, 13600, 13552, 13504, 13456, 13408, 13360, 13312, 13264, 13216, 13168, 13120, 13072, 13024, 12976,
12928, 12880, 12832, 12784, 12736, 12696, 12656, 12608, 12560, 12512, 12464, 12424, 12384, 12336, 12288, 12240,
12192, 12152, 12112, 12072, 12032, 11984, 11936, 11896, 11856, 11816, 11776, 11728, 11680, 11640, 11600, 11560,
11520, 11480, 11440, 11392, 11344, 11304, 11264, 11224, 11184, 11144, 11104, 11064, 11024, 10984, 10944, 10896,
10848, 10824, 10800, 10760, 10720, 10680, 10640, 10600, 10560, 10520, 10480, 10448, 10416, 10376, 10336, 10288,
10240, 10208, 10176, 10144, 10112, 10080, 10048, 10008,  9968,  9936,  9904,  9864,  9824,  9792,  9760,  9712,
9664,  9640,  9616,  9584,  9552,  9512,  9472,  9440,  9408,  9376,  9344,  9312,  9280,  9240,  9200,  9160,
9120,  9096,  9072,  9040,  9008,  8976,  8944,  8912,  8880,  8848,  8816,  8784,  8752,  8720,  8688,  8648,
8608,  8584,  8560,  8536,  8512,  8480,  8448,  8416,  8384,  8352,  8320,  8288,  8256,  8232,  8208,  8168,
8128,  8104,  8080,  8056,  8032,  8000,  7968,  7936,  7904,  7880,  7856,  7824,  7792,  7768,  7744,  7712,
7680,  7656,  7632,  7608,  7584,  7552,  7520,  7496,  7472,  7440,  7408,  7384,  7360,  7336,  7312,  7284,
7256,  7228,  7200,  7176,  7152,  7124,  7096,  7072,  7048,  7024,  7000,  6972,  6944,  6920,  6896,  6872,
6848,  6824,  6800,  6776,  6752,  6728,  6704,  6680,  6656,  6632,  6608,  6584,  6560,  6536,  6512,  6488,
6464,  6440,  6416,  6392,  6368,  6348,  6328,  6304,  6280,  6256,  6232,  6212,  6192,  6168,  6144,  6120,
6096,  6076,  6056,  6036,  6016,  5992,  5968,  5948,  5928,  5908,  5888,  5864,  5840,  5820,  5800,  5780,
5760,  5740,  5720,  5696,  5672,  5652,  5632,  5612,  5592,  5572,  5552,  5532,  5512,  5492,  5472,  5448,
5424,  5412,  5400,  5380,  5360,  5340,  5320,  5300,  5280,  5260,  5240,  5224,  5208,  5188,  5168,  5144,
5120,  5104,  5088,  5072,  5056,  5040,  5024,  5004,  4984,  4968,  4952,  4932,  4912,  4896,  4880,  4856,
4832,  4820,  4808,  4792,  4776,  4756,  4736,  4720,  4704,  4688,  4672,  4656,  4640,  4620,  4600,  4580,
4560,  4548,  4536,  4520,  4504,  4488,  4472,  4456,  4440,  4424,  4408,  4392,  4376,  4360,  4344,  4324,
4304,  4292,  4280,  4268,  4256,  4240,  4224,  4208,  4192,  4176,  4160,  4144,  4128,  4116,  4104,  4084,
4064,  4052,  4040,  4028,  4016,  4000,  3984,  3968,  3952,  3940,  3928,  3912,  3896,  3884,  3872,  3856,
3840,  3828,  3816,  3804,  3792,  3776,  3760,  3748,  3736,  3720,  3704,  3692,  3680,  3668,  3656,  3642,
3628,  3614,  3600,  3588,  3576,  3562,  3548,  3536,  3524,  3512,  3500,  3486,  3472,  3460,  3448,  3436,
3424,  3412,  3400,  3388,  3376,  3364,  3352,  3340,  3328,  3316,  3304,  3292,  3280,  3268,  3256,  3244,
3232,  3220,  3208,  3196,  3184,  3174,  3164,  3152,  3140,  3128,  3116,  3106,  3096,  3084,  3072,  3060,
3048,  3038,  3028,  3018,  3008,  2996,  2984,  2974,  2964,  2954,  2944,  2932,  2920,  2910,  2900,  2890,
2880,  2870,  2860,  2848,  2836,  2826,  2816,  2806,  2796,  2786,  2776,  2766,  2756,  2746,  2736,  2724,
2712,  2706,  2700,  2690,  2680,  2670,  2660,  2650,  2640,  2630,  2620,  2612,  2604,  2594,  2584,  2572,
2560,  2552,  2544,  2536,  2528,  2520,  2512,  2502,  2492,  2484,  2476,  2466,  2456,  2448,  2440,  2428,
2416,  2410,  2404,  2396,  2388,  2378,  2368,  2360,  2352,  2344,  2336,  2328,  2320,  2310,  2300,  2290,
2280,  2274,  2268,  2260,  2252,  2244,  2236,  2228,  2220,  2212,  2204,  2196,  2188,  2180,  2172,  2162,
2152,  2146,  2140,  2134,  2128,  2120,  2112,  2104,  2096,  2088,  2080,  2072,  2064,  2058,  2052,  2042,
2032,  2026,  2020,  2014,  2008,  2000,  1992,  1984,  1976,  1970,  1964,  1956,  1948,  1942,  1936,  1928,
1920,  1914,  1908,  1902,  1896,  1888,  1880,  1874,  1868,  1860,  1852,  1846,  1840,  1834,  1828,  1821,
1814,  1807,  1800,  1794,  1788,  1781,  1774,  1768,  1762,  1756,  1750,  1743,  1736,  1730,  1724,  1718,
1712,  1706,  1700,  1694,  1688,  1682,  1676,  1670,  1664,  1658,  1652,  1646,  1640,  1634,  1628,  1622,
1616,  1610,  1604,  1598,  1592,  1587,  1582,  1576,  1570,  1564,  1558,  1553,  1548,  1542,  1536,  1530,
1524,  1519,  1514,  1509,  1504,  1498,  1492,  1487,  1482,  1477,  1472,  1466,  1460,  1455,  1450,  1445,
1440,  1435,  1430,  1424,  1418,  1413,  1408,  1403,  1398,  1393,  1388,  1383,  1378,  1373,  1368,  1362,
1356,  1353,  1350,  1345,  1340,  1335,  1330,  1325,  1320,  1315,  1310,  1306,  1302,  1297,  1292,  1286,
1280,  1276,  1272,  1268,  1264,  1260,  1256,  1251,  1246,  1242,  1238,  1233,  1228,  1224,  1220,  1214,
1208,  1205,  1202,  1198,  1194,  1189,  1184,  1180,  1176,  1172,  1168,  1164,  1160,  1155,  1150,  1145,
1140,  1137,  1134,  1130,  1126,  1122,  1118,  1114,  1110,  1106,  1102,  1098,  1094,  1090,  1086,  1081,
1076,  1073,  1070,  1067,  1064,  1060,  1056,  1052,  1048,  1044,  1040,  1036,  1032,  1029,  1026,  1021,
1016,  1013,  1010,  1007,  1004,  1000,   996,   992,   988,   985,   982,   978,   974,   971,   968,   964,
960,   957,   954,   951,   948,   944,   940,   937,   934,   930,   926,   923,   920,   917,   914,   910,
907,   903,   900,   897,   894,   890,   887,   884,   881,   878,   875,   871,   868,   865,   862,   859,
856,   853,   850,   847,   844,   841,   838,   835,   832,   829,   826,   823,   820,   817,   814,   811,
808,   805,   802,   799,   796,   793,   791,   788,   785,   782,   779,   776,   774,   771,   768,   765,
762,   759,   757,   754,   752,   749,   746,   743,   741,   738,   736,   733,   730,   727,   725,   722,
720,   717,   715,   712,   709,   706,   704,   701,   699,   696,   694,   691,   689,   686,   684,   681,
678,   676,   675,   672,   670,   667,   665,   662,   660,   657,   655,   653,   651,   648,   646,   643,
640,   638,   636,   634,   632,   630,   628,   625,   623,   621,   619,   616,   614,   612,   610,   607,
604,   602,   601,   599,   597,   594,   592,   590,   588,   586,   584,   582,   580,   577,   575,   572,
570,   568,   567,   565,   563,   561,   559,   557,   555,   553,   551,   549,   547,   545,   543,   540,
538,   536,   535,   533,   532,   530,   528,   526,   524,   522,   520,   518,   516,   514,   513,   510,
508,   506,   505,   503,   502,   500,   498,   496,   494,   492,   491,   489,   487,   485,   484,   482,
480,   478,   477,   475,   474,   472,   470,   468,   467,   465,   463,   461,   460,   458,   457,   455,
453,   451,   450,   448,   447,   445,   443,   441,   440,   438,   437,   435,   434,   432,   431,   429,
428,   426,   425,   423,   422,   420,   419,   417,   416,   414,   413,   411,   410,   408,   407,   405,
404,   402,   401,   399,   398,   396,   395,   393,   392,   390,   389,   388,   387,   385,   384,   382,
381,   379,   378,   377,   376,   374,   373,   371,   370,   369,   368,   366,   365,   363,   362,   361,
360,   358,   357,   355,   354,   353,   352,   350,   349,   348,   347,   345,   344,   343,   342,   340,
339,   338,   337,   336,   335,   333,   332,   331,   330,   328,   327,   326,   325,   324,   323,   321,
320,   319,   318,   317,   316,   315,   314,   312,   311,   310,   309,   308,   307,   306,   305,   303,
302,   301,   300,   299,   298,   297,   296,   295,   294,   293,   292,   291,   290,   288,   287,   286,
285,   284,   283,   282,   281,   280,   279,   278,   277,   276,   275,   274,   273,   272,   271,   270,
269,   268,   267,   266,   266,   265,   264,   263,   262,   261,   260,   259,   258,   257,   256,   255,
254,   253,   252,   251,   251,   250,   249,   248,   247,   246,   245,   244,   243,   242,   242,   241,
240,   239,   238,   237,   237,   236,   235,   234,   233,   232,   231,   230,   230,   229,   228,   227,
227,   226,   225,   224,   223,   222,   222,   221,   220,   219,   219,   218,   217,   216,   215,   214,
214,   213,   212,   211,   211,   210,   209,   208,   208,   207,   206,   205,   205,   204,   203,   202,
202,   201,   200,   199,   199,   198,   198,   197,   196,   195,   195,   194,   193,   192,   192,   191,
190,   189,   189,   188,   188,   187,   186,   185,   185,   184,   184,   183,   182,   181,   181,   180,
180,   179,   179,   178,   177,   176,   176,   175,   175,   174,   173,   172,   172,   171,   171,   170,
169,   169,   169,   168,   167,   166,   166,   165,   165,   164,   164,   163,   163,   162,   161,   160,
160,   159,   159,   158,   158,   157,   157,   156,   156,   155,   155,   154,   153,   152,   152,   151,
151,   150,   150,   149,   149,   148,   148,   147,   147,   146,   146,   145,   145,   144,   144,   143,
142,   142,   142,   141,   141,   140,   140,   139,   139,   138,   138,   137,   137,   136,   136,   135,
134,   134,   134,   133,   133,   132,   132,   131,   131,   130,   130,   129,   129,   128,   128,   127,
127,   126,   126,   125,   125,   124,   124,   123,   123,   123,   123,   122,   122,   121,   121,   120,
120,   119,   119,   118,   118,   117,   117,   117,   117,   116,   116,   115,   115,   114,   114,   113,
113,   112,   112,   112,   112,   111,   111,   110,   110,   109,   109,   108,   108,   108,   108,   107,
107,   106,   106,   105,   105,   105,   105,   104,   104,   103,   103,   102,   102,   102,   102,   101,
101,   100,   100,    99,    99,    99,    99,    98,    98,    97,    97,    97,    97,    96,    96,    95,
95,    95,    95,    94,    94,    93,    93,    93,    93,    92,    92,    91,    91,    91,    91,    90,
90,    89,    89,    89,    89,    88,    88,    87,    87,    87,    87,    86,    86,    85,    85,    85,
85,    84,    84,    84,    84,    83,    83,    82,    82,    82,    82,    81,    81,    81,    81,    80,
80,    79,    79,    79,    79,    78,    78,    78,    78,    77,    77,    77,    77,    76,    76,    75,
75,    75,    75,    75,    75,    74,    74,    73,    73,    73,    73,    72,    72,    72,    72,    71,
71,    71,    71,    70,    70,    70,    70,    69,    69,    69,    69,    68,    68,    68,    68,    67,
67,    67,    67,    66,    66,    66,    66,    65,    65,    65,    65,    64,    64,    64,    64,    63,
63,    63,    63,    63,    63,    62,    62,    62,    62,    61,    61,    61,    61,    60,    60,    60,
60,    60,    60,    59,    59,    59,    59,    58,    58,    58,    58,    57,    57,    57,    57,    57,
57,    56,    56,    56,    56,    55,    55,    55,    55,    55,    55,    54,    54,    54,    54,    53,
53,    53,    53,    53,    53,    52,    52,    52,    52,    52,    52,    51,    51,    51,    51,    50,
50,    50,    50,    50,    50,    49,    49,    49,    49,    49,    49,    48,    48,    48,    48,    48,
48,    47,    47,    47,    47,    47,    47,    46,    46,    46,    46,    46,    46,    45,    45,    45,
45,    45,    45,    44,    44,    44,    44,    44,    44,    43,    43,    43,    43,    43,    43,    42,
42,    42,    42,    42,    42,    42,    42,    41,    41,    41,    41,    41,    41,    40,    40,    40,
40,    40,    40,    39,    39,    39,    39,    39,    39,    39,    39,    38,    38,    38,    38,    38,
38,    38,    38,    37,    37,    37,    37,    37,    37,    36,    36,    36,    36,    36,    36,    36,
36,    35,    35,    35,    35,    35,    35,    35,    35,    34,    34,    34,    34,    34,    34,    34,
34,    33,    33,    33,    33,    33,    33,    33,    33,    32,    32,    32,    32,    32,    32,    32,
32,    32,    32,    31,    31,    31,    31,    31,    31,    31,    31,    30,    30,    30,    30,    30,
30,    30,    30,    30,    30,    29,    29,    29,    29,    29,    29,    29,    29,    29,    29,    22,
16,     8,     0,    16,    32,    24,    16,     8,     0,    16,    32,    24,    16,     8,     0,     0
// the last 17 values are off (but identical to FT2) because of a bug in how FT2 calculates this table
];

/* ----------------------------------------------------------------------- */
/*                           AUDIO MIXER TABLES                            */
/* ----------------------------------------------------------------------- */

pub const PANNING_TAB: [u32;257] = // bit-exact to FT2 table
[
    0, 4096, 5793, 7094, 8192, 9159,10033,10837,11585,12288,12953,13585,14189,14768,15326,15864,
16384,16888,17378,17854,18318,18770,19212,19644,20066,20480,20886,21283,21674,22058,22435,22806,
23170,23530,23884,24232,24576,24915,25249,25580,25905,26227,26545,26859,27170,27477,27780,28081,
28378,28672,28963,29251,29537,29819,30099,30377,30652,30924,31194,31462,31727,31991,32252,32511,
32768,33023,33276,33527,33776,34024,34270,34514,34756,34996,35235,35472,35708,35942,36175,36406,
36636,36864,37091,37316,37540,37763,37985,38205,38424,38642,38858,39073,39287,39500,39712,39923,
40132,40341,40548,40755,40960,41164,41368,41570,41771,41972,42171,42369,42567,42763,42959,43154,
43348,43541,43733,43925,44115,44305,44494,44682,44869,45056,45242,45427,45611,45795,45977,46160,
46341,46522,46702,46881,47059,47237,47415,47591,47767,47942,48117,48291,48465,48637,48809,48981,
49152,49322,49492,49661,49830,49998,50166,50332,50499,50665,50830,50995,51159,51323,51486,51649,
51811,51972,52134,52294,52454,52614,52773,52932,53090,53248,53405,53562,53719,53874,54030,54185,
54340,54494,54647,54801,54954,55106,55258,55410,55561,55712,55862,56012,56162,56311,56459,56608,
56756,56903,57051,57198,57344,57490,57636,57781,57926,58071,58215,58359,58503,58646,58789,58931,
59073,59215,59357,59498,59639,59779,59919,60059,60199,60338,60477,60615,60753,60891,61029,61166,
61303,61440,61576,61712,61848,61984,62119,62254,62388,62523,62657,62790,62924,63057,63190,63323,
63455,63587,63719,63850,63982,64113,64243,64374,64504,64634,64763,64893,65022,65151,65279,65408,
65536
];

// #[derive(Eq, PartialEq, Clone, Copy, Debug)]
// pub enum TableType {
//     LinearFrequency,
//     AmigaFrequency
// }
//
// pub struct AudioTables {
//     pub periods:        [i16; 1936],
//     pub d_period2hz_tab:  [f64; 65536],
// }
//
// pub const AmigaTables: AudioTables = AudioTables { periods: LINEAR_PERIODS, d_period2hz_tab: [0.0f64; 65536] };
// pub const LinearTables: AudioTables = AudioTables{ periods: AMIGA_PERIODS, d_period2hz_tab: [0.0f64; 65536] };
//
//
//
// use core::fmt;
// impl fmt::Display for AudioTables {
//     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
//         let mut first = true;
//         let mut i = 0;
//         write!(f, "    AudioTables   = {{ periods: [");
//         // for period in self.periods.iter().cloned() {
//         //     if first { first = false; } else { write!(f, ",\t")?; }
//         //     write!(f, "{}", period)?;
//         //     if i > 16 {
//         //         i = 0;
//         //         write!(f, "\n\t\t\t");
//         //     }
//         // }
//         write!(f, "],");
//         write!(f, "\t\t\tdPeriod2HzTab: [\n");
//         for hz in self.d_period2hz_tab.iter().cloned() {
//             if first { first = false; } else { write!(f, ",\t")?; }
//             write!(f, "{:.9}", hz)?;
//             if i > 10 {
//                 i = 0;
//                 write!(f, "\n\t\t\t");
//             } else {
//                 i +=1;
//             }
//         }
//         write!(f, "]}};");
//
//         Ok(())
//     }
// }
//
//
// #[cfg(test)]
// mod tests {
//     use crate::tables::{AudioTables, LINEAR_PERIODS, AMIGA_PERIODS};
//     use std::fs::File;
//     use std::io::Write;
//
//     fn log(i: usize) -> f64 {
//         (i as f64 / 768.0).exp2() * (8363.0 * 256.0)
//     }
//
//     #[test]
//     fn calc_tables_linear() // taken directly from ft2clone
//     {
//         let mut result = AudioTables { periods: LINEAR_PERIODS, d_period2hz_tab: [0.0f64; 65536] };
//         result.d_period2hz_tab[0] = 0.0; // in FT2, a period of 0 yields 0Hz
//
//         // linear periods
//         for i in 1..65536 {
//             let invPeriod = (12 * 192 * 4 as u16).wrapping_sub(i as u16); // this intentionally overflows uint16_t to be accurate to FT2
//             let octave = invPeriod as u32 / 768;
//             let period = invPeriod as u32 % 768;
//             let bitshift = (14u32.wrapping_sub(octave)) & 0x1F; // 100% accurate to FT2
//
//             result.d_period2hz_tab[i] = log(period as usize) / (1 << bitshift) as f64;
//         }
//         let mut file = File::create("linear.txt").unwrap();
//         file.write_all(format!("pub const LinearTables: {}", result).as_ref());
//     }
//
//     #[test]
//     fn calc_tables_amiga() // taken directly from ft2clone
//     {
//         let mut result = AudioTables { periods: AMIGA_PERIODS, d_period2hz_tab: [0.0f64; 65536] };
//         result.d_period2hz_tab[0] = 0.0; // in FT2, a period of 0 yields 0Hz
//         // Amiga periods
//         for i in 1..65536 {
//             result.d_period2hz_tab[i] = (8363.0 * 1712.0) / i as f64;
//         }
//         let mut file = File::create("amiga.txt").unwrap();
//         file.write_all(format!("pub const AmigaTables:  {}", result).as_ref());
//     }
// }


fn log(i: usize) -> f64 {
    (i as f64 / 768.0).exp2() * (8363.0 * 256.0)
}

fn log_table() -> Box<[f64; 768]> {
    let mut result = box [0.0f64; 768];
    for i in 0..768 {
        result[i] = log(i);
    }
    result
}

lazy_static! {
pub static ref D_LOG_TAB: Box<[f64; 768]> = log_table();
}

#[derive(Eq, PartialEq, Clone, Copy, Debug)]
pub enum TableType {
    LinearFrequency,
    AmigaFrequency
}

pub struct AudioTables {
    pub periods:        [u16; 1936],
    pub d_period2hz_tab:  [f64; 65536],
}

impl AudioTables {
    fn calc_tables_linear() -> Box<AudioTables> // taken directly from ft2clone
    {
        let mut result = box Self { periods: LINEAR_PERIODS, d_period2hz_tab: [0.0f64; 65536] };
        result.d_period2hz_tab[0] = 0.0; // in FT2, a period of 0 yields 0Hz
        // linear periods
        for i in 1..65536 {
            let inv_period = (12 * 192 * 4 as u16).wrapping_sub(i as u16); // this intentionally overflows uint16_t to be accurate to FT2
            let octave = inv_period as u32 / 768;
            let period = inv_period as u32 % 768;
            let bitshift = (14u32.wrapping_sub(octave)) & 0x1F; // 100% accurate to FT2

            result.d_period2hz_tab[i] = D_LOG_TAB[period as usize] / (1 << bitshift) as f64;
        }
        result
    }

    fn calc_tables_amiga() -> Box<AudioTables> // taken directly from ft2clone
    {
        let mut result = box Self { periods: AMIGA_PERIODS, d_period2hz_tab: [0.0f64; 65536] };
        result.d_period2hz_tab[0] = 0.0; // in FT2, a period of 0 yields 0Hz
        // Amiga periods
        for i in 1..65536 {
            result.d_period2hz_tab[i] = (8363.0 * 1712.0) / i as f64;
        }
        result
    }
}

lazy_static! {
pub static ref AMIGA_TABLES:   Box<AudioTables>   = AudioTables::calc_tables_amiga();
pub static ref LINEAR_TABLES:  Box<AudioTables>   = AudioTables::calc_tables_linear();
}
