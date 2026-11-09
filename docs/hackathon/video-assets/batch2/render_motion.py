#!/usr/bin/env python3
"""Generate decorative technical B-roll. Illustrative motion only, NOT recorded execution."""
from pathlib import Path
from math import sin,cos,pi
import subprocess
from PIL import Image,ImageDraw,ImageFilter

def create_motion_clips(root:Path):
    png=root/"png"
    video=root/"video"
    video.mkdir(exist_ok=True)
    scenes=[
      ("13_x3bc_bytecode.png","X3BC_Bytecode_Pulse_4s.mp4",
       [(790,575),(1045,575),(1430,575)],(72,232,249)),
      ("18_near_guard_decision_fork.png","X3Shield_Guard_Decision_4s.mp4",
       [(670,553),(1038,553),(1261,548),(1370,530)],(101,241,194)),
      ("20_tidepool_route.png","Tidepool_Trusted_Relay_4s.mp4",
       [(540,581),(945,581),(1368,581)],(255,188,107)),
    ]
    w,h=1280,720
    for source,outname,points,rgb in scenes:
        raw=Image.open(png/source).convert("RGBA").resize((w,h),Image.Resampling.LANCZOS)
        points=[(x*2/3,y*2/3) for x,y in points]
        n=96
        out=video/outname
        cmd=["ffmpeg","-hide_banner","-loglevel","error","-y","-f","rawvideo",
             "-pix_fmt","rgb24","-s","1280x720","-r","24","-i","-","-an",
             "-c:v","libx264","-preset","ultrafast","-crf","24",
             "-pix_fmt","yuv420p","-movflags","+faststart",str(out)]
        proc=subprocess.Popen(cmd,stdin=subprocess.PIPE)
        try:
            for i in range(n):
                t=i/n
                seg=t*(len(points)-1)
                a=min(int(seg),len(points)-2)
                f=min(1,seg-a)
                x=points[a][0]*(1-f)+points[a+1][0]*f
                y=points[a][1]*(1-f)+points[a+1][1]*f
                pulse=7+4*sin(t*2*pi*2)**2
                layer=Image.new("RGBA",(w,h),(0,0,0,0))
                d=ImageDraw.Draw(layer,"RGBA")
                for radius,alpha in [(65,18),(43,29),(25,66)]:
                    d.ellipse((x-radius,y-radius,x+radius,y+radius),
                              fill=(*rgb,alpha))
                d.ellipse((x-pulse,y-pulse,x+pulse,y+pulse),
                          fill=(*rgb,225),outline=(255,255,255,240),width=2)
                # Decorative arc at the highlight point; no transaction tracing is implied.
                ring=25+(i%24)*1.3
                d.ellipse((x-ring,y-ring,x+ring,y+ring),
                          outline=(*rgb,int(115*(1-(i%24)/24))),width=3)
                glow=layer.filter(ImageFilter.GaussianBlur(14))
                frame=Image.alpha_composite(Image.alpha_composite(raw,glow),layer)
                proc.stdin.write(frame.convert("RGB").tobytes())
        finally:
            if proc.stdin and not proc.stdin.closed: proc.stdin.close()
        code=proc.wait()
        assert code==0,(outname,code)
        subprocess.run(["ffprobe","-v","error","-show_entries","format=duration",
                       "-of","default=noprint_wrappers=1",str(out)],check=True)
        print("MOTION_OK",out.name,out.stat().st_size,flush=True)

if __name__=="__main__":
    create_motion_clips(Path(__file__).resolve().parent)
