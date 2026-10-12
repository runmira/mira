#!/usr/bin/env python3
"""Render the DMG background at the configured 720 × 480 Finder size."""
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont
ROOT = Path(__file__).resolve().parents[1]
S = 2
im = Image.new('RGB', (720*S, 480*S), '#F7F5F0')
d = ImageDraw.Draw(im)
def font(size):
    return ImageFont.truetype('/System/Library/Fonts/Supplemental/Arial.ttf', size*S)
def text(x,y,value,size,color):
    d.text((x*S,y*S),value,font=font(size),fill=color)
text(44,36,'Mira',21,'#7773D8')
text(44,90,'Make it yours.',38,'#28272C')
text(44,146,'Drag Mira into Applications.',16,'#74717C')
# A loose curve connects the native Finder icons, without boxed panels.
points=[]
for i in range(101):
    t=i/100
    x=(1-t)**3*305+3*(1-t)**2*t*326+3*(1-t)*t*t*376+t**3*413
    y=(1-t)**3*272+3*(1-t)**2*t*227+3*(1-t)*t*t*300+t**3*260
    points.append((round(x*S),round(y*S)))
d.line(points, fill='#A49BE5',width=3*S)
d.line([(397*S,254*S),(414*S,259*S),(405*S,275*S)],fill='#A49BE5',width=3*S)
text(44,427,'runmira.com',13,'#96919F')
im.resize((720,480),Image.Resampling.LANCZOS).save(ROOT/'apps/desktop/installer/background.png')
