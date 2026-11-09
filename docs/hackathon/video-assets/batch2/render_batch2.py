#!/usr/bin/env python3
"""X3Lang video asset renderer: 16 SVGs -> 1080p PNG, short reels, ZIP, and gallery."""
from pathlib import Path
from xml.etree import ElementTree
import subprocess, zipfile, json, hashlib
import cairosvg
from PIL import Image
root=Path(__file__).resolve().parent
svg=root/"svg"; png=root/"png"; vid=root/"video"
png.mkdir(exist_ok=True); vid.mkdir(exist_ok=True)
paths=sorted(svg.glob("*.svg"))
assert len(paths)==16, f"Expected 16 SVGs, found {len(paths)}"
for src in paths:
    el=ElementTree.parse(src).getroot()
    assert el.tag.endswith("svg") and el.attrib["viewBox"]=="0 0 1920 1080"
    dst=png/(src.stem+".png")
    cairosvg.svg2png(url=str(src),write_to=str(dst),output_width=1920,output_height=1080)
    with Image.open(dst) as check: assert check.size==(1920,1080)
    print("PNG_OK",dst.name,dst.stat().st_size,flush=True)
images=sorted(png.glob("*.png"))
sheet=Image.new("RGB",(1600,900),(6,17,34))
for i,f in enumerate(images):
    shot=Image.open(f).convert("RGB").resize((400,225),Image.Resampling.LANCZOS)
    sheet.paste(shot,((i%4)*400,(i//4)*225))
sheet.save(root/"contact_sheet.png",optimize=True)
def make_video(name, indexes, duration=2.7,fade=.5):
    sources=[images[i] for i in indexes]
    args=[]
    for src in sources: args+=["-loop","1","-framerate","24","-t",str(duration),"-i",str(src)]
    graph=[f"[{i}:v]scale=1280:720,setsar=1,format=yuv420p[v{i}]" for i in range(len(sources))]
    total=duration; last="v0"
    for i in range(1,len(sources)):
        label=f"x{i}"
        graph.append(f"[{last}][v{i}]xfade=transition=fade:duration={fade}:offset={total-fade:.2f}[{label}]")
        last=label;total+=duration-fade
    dst=vid/name
    cmd=["ffmpeg","-hide_banner","-loglevel","error","-y",*args,"-filter_complex",
         ";".join(graph),"-map",f"[{last}]","-r","24","-pix_fmt","yuv420p",
         "-c:v","libx264","-preset","ultrafast","-crf","24",
         "-movflags","+faststart","-t",f"{total:.2f}",str(dst)]
    print("ENCODING",name,flush=True)
    try:
        subprocess.run(cmd,check=True)
    except subprocess.CalledProcessError:
        print("Crossfade encoder failed; using simple cut-safe fallback",flush=True)
        concat=vid/(name+".ffconcat")
        rows=["ffconcat version 1.0"]
        for f in sources:
            rows.extend(["file '"+str(f)+"'",f"duration {duration}"])
        rows.append("file '"+str(sources[-1])+"'")
        concat.write_text("\n".join(rows)+"\n",encoding="utf-8")
        subprocess.run(["ffmpeg","-hide_banner","-loglevel","error","-y","-f","concat",
                        "-safe","0","-i",str(concat),"-vf","scale=1280:720,format=yuv420p",
                        "-r","24","-c:v","libx264","-preset","ultrafast","-crf","24",
                        "-pix_fmt","yuv420p","-movflags","+faststart",str(dst)],check=True)
    subprocess.run(["ffprobe","-v","error","-show_entries","format=duration,size",
                    "-of","default=noprint_wrappers=1",str(dst)],check=True)
    print("VIDEO_OK",dst.name,dst.stat().st_size,flush=True)
make_video("X3Lang_Technical_Reel.mp4",range(16))
make_video("X3Lang_Compiler_X3BC_X3VM.mp4",range(6),3.0,.5)
make_video("X3Shield_Near_Tidepool_Proof.mp4",range(8,16),3.0,.5)
from render_motion import create_motion_clips
create_motion_clips(root)
cards="".join('<article><a href="png/'+p.name+'"><img loading="lazy" src="png/'+p.name+'"></a><p>'+
              p.stem.replace("_"," ").title()+'<br><a href="svg/'+p.stem+'.svg">SVG master</a></p></article>' for p in images)
videos="".join('<h3>'+p.stem.replace("_"," ")+'</h3><video controls preload="metadata" src="video/'+p.name+'"></video>' for p in sorted(vid.glob("*.mp4")))
html='''<!doctype html><html lang="en"><meta charset="utf-8"><title>X3Lang batch 2 video</title>
<style>body{margin:0;padding:35px;background:#09152a;color:#eef5ff;font:17px system-ui}
h1{color:#6be6f7}main{display:grid;grid-template-columns:repeat(auto-fit,minmax(380px,1fr));gap:20px}
article{background:#142641;border:1px solid #456588;border-radius:12px;overflow:hidden}
article img{width:100%}article p{padding:10px 20px}a{color:#7fe7fa}
video{width:min(95%,840px);border-radius:13px}aside{background:#342b3e;padding:15px;border-radius:10px}</style>
<h1>X3LANG / X3 SHIELD — TECHNICAL VISUALS</h1>
<aside>The X3BC VM path and the NEAR Guard path are different. The funded Tidepool
testnet pilot is separate from the official 1Click signed quote demo.</aside><section>'''+videos+'</section><h2>16 editable scenes</h2><main>'+cards+'</main></html>'
(root/"index.html").write_text(html,encoding="utf-8")
items=[*sorted(svg.glob("*.svg")),*images,*sorted(vid.glob("*.mp4")),root/"contact_sheet.png",root/"index.html",root/"README.md",root/"render_batch2.py",root/"render_motion.py"]
out=root.parent/"X3Lang_Batch2_Technical_Assets.zip"
with zipfile.ZipFile(out,"w",zipfile.ZIP_DEFLATED,compresslevel=6) as z:
    for f in items:z.write(f,arcname=f.relative_to(root.parent))
with zipfile.ZipFile(out) as z:assert z.testzip() is None
manifest={"scene_count":len(paths),"videos":[p.name for p in sorted(vid.glob("*.mp4"))],
          "zip_path":str(out),"zip_bytes":out.stat().st_size,"sha256":hashlib.sha256(out.read_bytes()).hexdigest()}
(root/"manifest.json").write_text(json.dumps(manifest,indent=2))
print("FINISHED",json.dumps(manifest),flush=True)
