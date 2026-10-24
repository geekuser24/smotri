"""Run once on the owner's server. Extract only known catalog constants, never execute bot code."""
import ast
import json
import os
import secrets
import shutil
from pathlib import Path
source=Path(os.environ['SMOTRI_BOT_SOURCE']).resolve()  # Private bot directory, supplied only on the server.
dest=Path('/var/lib/smotri/packs');dest.mkdir(parents=True,exist_ok=True)
private=Path('/etc/smotri');private.mkdir(mode=0o750,exist_ok=True)
config_path=private/'config.json'
if config_path.exists(): raise SystemExit('Configuration already exists; preserving it. Update intentionally.')
values={}
for n in ast.parse((source/'geekmp4.py').read_text()).body:
    if isinstance(n,ast.Assign):
        for x in n.targets:
            if isinstance(x,ast.Name) and x.id in ('TOKEN','MONTAGE_PACKS','PLUGINS_LINK'):
                values[x.id]=ast.literal_eval(n.value)
items=[]
video={
'pack_6':('transition.jpg','TRANSITIONS PRESETS.prfpset','https://youtu.be/OIWKgCqxZxs'),
'pack_7':('dynamic.jpg','@acekillthat.mp4 DYNAMIC FX PACK.prfpset','https://youtu.be/gtZyvq8pQeo'),
'pack_9':('gunshot.jpg',None,'https://youtu.be/3nl6KZmZ6SU')}
cover_dir=Path('/opt/smotri/site/static/covers');cover_dir.mkdir(exist_ok=True)
for category,packs in values['MONTAGE_PACKS'].items():
    for id,p in packs.items():
        item={'id':id,'name':p['name'],'description':p['description'],'category':category,'format':'КОЛЛЕКЦИЯ','video':'https://www.youtube.com/@akt.mp4'}
        cover=Path(p.get('image_path','')).name
        if id in video:
            cover,filename,item['video']=video[id]
            if filename:
                target=dest/filename;shutil.copy2(source/filename,target);os.chmod(target,0o640);item['file']=str(target);item['format']='PREMIERE PRO · .PRFPSET'
            else:item['url']='https://drive.google.com/file/d/1OwOwPpqVHQOWX8QwyFJ7bfXllUkGx1hL/view'
        if cover:
            shutil.copy2(source/'packscover'/cover,cover_dir/cover);item['cover']='/static/covers/'+cover
        items.append(item)
items.append({'id':'plugins','name':'Сборник плагинов','description':'Подборка из видео AKT.MP4','category':'plugins','video':'https://youtu.be/svd4k8NvSWI','url':values['PLUGINS_LINK']})
cfg={'origin':'https://smotri.cc','session_key':secrets.token_hex(48),'bot_token':values['TOKEN'],'bot_username':'Geekmp4_bot','channel_id':'-1001744393780','channel_url':'https://t.me/aktmp4','pack_root':str(dest),'catalog':items,'releases':[]}
fd=os.open(config_path,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o640)
with os.fdopen(fd,'w') as f:json.dump(cfg,f,ensure_ascii=False,indent=2)
print('Catalog created:',len(items),'items. Secret values kept only in /etc/smotri/config.json.')
