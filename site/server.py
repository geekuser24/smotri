"""Smotri catalog: secrets and private download paths live outside the source tree."""
import base64
import hashlib
import hmac
import io
import json
import os
import secrets
import time
from pathlib import Path
from urllib.parse import quote

import httpx
import bot_auth
from fastapi import FastAPI, HTTPException, Request
from fastapi.responses import FileResponse, JSONResponse, RedirectResponse, Response
from fastapi.staticfiles import StaticFiles
from PIL import Image, UnidentifiedImageError

ROOT = Path(__file__).parent
CONFIG = Path(os.environ.get('SMOTRI_CONFIG', '/etc/smotri/config.json'))
app = FastAPI(docs_url=None, redoc_url=None, openapi_url=None)
Image.MAX_IMAGE_PIXELS = 24_000_000

def config():
    return json.loads(CONFIG.read_text())

def sign(payload, key):
    raw = base64.urlsafe_b64encode(json.dumps(payload, separators=(',', ':')).encode()).decode().rstrip('=')
    return raw + '.' + hmac.new(key.encode(), raw.encode(), hashlib.sha256).hexdigest()

def verify(value, key):
    try:
        if len(value) > 8192: return None
        raw, sig = value.split('.')
        expected = hmac.new(key.encode(), raw.encode(), hashlib.sha256).hexdigest()
        if not hmac.compare_digest(sig, expected): return None
        body = json.loads(base64.urlsafe_b64decode(raw + '=' * (-len(raw) % 4)))
        if not isinstance(body, dict) or not isinstance(body.get('exp'), int) or body['exp'] < time.time(): return None
        return body
    except (ValueError, TypeError, UnicodeError): return None

def telegram_valid(data, token, now=None):
    now = time.time() if now is None else now
    try:
        if not isinstance(data, dict) or len(data) > 12: return False
        if any(not isinstance(k, str) or not isinstance(v, (str, int)) or len(str(v)) > 2048 for k, v in data.items()): return False
        age = now - int(data['auth_date'])
        if age < -30 or age > 300 or int(data['id']) <= 0: return False
        check = '\n'.join(f'{k}={data[k]}' for k in sorted(data) if k != 'hash')
        secret = hashlib.sha256(token.encode()).digest()
        return hmac.compare_digest(hmac.new(secret, check.encode(), hashlib.sha256).hexdigest(), str(data.get('hash', '')))
    except (KeyError, TypeError, ValueError): return False

def same_origin(request, cfg):
    if request.headers.get('origin') != cfg['origin']: raise HTTPException(403, 'Обновите страницу и повторите действие.')

def session(request, cfg):
    return verify(request.cookies.get('smotri_session', ''), cfg['session_key']) or {}

def cookie(response, data, cfg):
    response.set_cookie('smotri_session', sign(data, cfg['session_key']), max_age=3600, secure=True, httponly=True, samesite='lax', path='/')
    return response

def item_by_id(cfg, item_id):
    item = next((i for i in cfg['catalog'] + cfg.get('releases', []) if i['id'] == item_id), None)
    if not item: raise HTTPException(404, 'Материал не найден.')
    if not item.get('file') and not item.get('url'): raise HTTPException(409, 'Архив готовится к публикации.')
    return item

@app.middleware('http')
async def security(request, call_next):
    response = await call_next(request)
    if request.url.path == '/' and request.query_params.get('preview'):
        response.headers['Clear-Site-Data'] = '"cache"'
    response.headers['X-Content-Type-Options'] = 'nosniff'
    if request.url.path.startswith('/static/'):
        response.headers['Cache-Control'] = 'public, max-age=0, must-revalidate'
    response.headers['Referrer-Policy'] = 'strict-origin-when-cross-origin'
    response.headers['X-Frame-Options'] = 'DENY'
    response.headers['Permissions-Policy'] = 'camera=(), microphone=(), geolocation=()'
    response.headers['Content-Security-Policy'] = "default-src 'self'; script-src 'self' https://telegram.org; frame-src https://oauth.telegram.org; img-src 'self' data: https://*.telegram.org https://i.ytimg.com; style-src 'self'; connect-src 'self'; base-uri 'none'; object-src 'none'; form-action 'self'; frame-ancestors 'none'"
    if request.url.path == '/' or request.url.path.startswith(('/api/', '/download/')): response.headers['Cache-Control'] = 'no-store'
    return response

@app.get('/api/catalog')
def catalog():
    cfg = config()
    return {'bot': cfg['bot_username'], 'channel': cfg['channel_url'], 'youtube': 'https://www.youtube.com/@akt.mp4',
            'items': [{**{k: i.get(k, '') for k in ('id', 'name', 'description', 'category', 'cover', 'format', 'video')}, 'available': bool(i.get('file') or i.get('url'))} for i in cfg['catalog']],
            'releases': [{**{k: i.get(k, '') for k in ('id', 'name', 'video')}, 'available': bool(i.get('file') or i.get('url'))} for i in cfg.get('releases', [])]}

@app.get('/api/session')
def current_session(request: Request):
    cfg = config(); s = session(request, cfg)
    return {'authenticated': bool(s.get('uid')), 'name': s.get('name', '')}

@app.post('/api/auth/start')
async def auth_start(request: Request):
    cfg=config();same_origin(request,cfg)
    try:
        data=await request.json()
        item=str(data.get('item',''))[:80]
    except (ValueError,AttributeError):raise HTTPException(400,'Некорректный запрос.')
    item_by_id(cfg,item)
    id,secret=bot_auth.create(item)
    r=JSONResponse({'url':'https://t.me/'+cfg['bot_username']+'?start=smotri_'+id,'code':id[:6].upper()})
    r.set_cookie('smotri_auth',sign({'id':id,'secret':secret,'exp':int(time.time())+600},cfg['session_key']),max_age=600,secure=True,httponly=True,samesite='lax',path='/api/auth')
    return r

@app.get('/api/auth/poll')
def auth_poll(request: Request):
    cfg=config();state=verify(request.cookies.get('smotri_auth',''),cfg['session_key']) or {}
    if not state.get('id') or not state.get('secret'):
        return {'state':'confirmed' if session(request,cfg).get('uid') else 'missing'}
    result=bot_auth.consume(state['id'],state['secret'])
    if result['state']!='confirmed':return {'state':result['state']}
    r=cookie(JSONResponse({'state':'confirmed','item':result['item']}),{'uid':result['uid'],'name':result['name'],'exp':int(time.time())+3600},cfg)
    r.delete_cookie('smotri_auth',path='/api/auth')
    return r

@app.post('/api/auth/bot')
async def auth_bot(request: Request):
    cfg=config()
    secret=cfg.get('bot_login_secret','')
    if not secret or not hmac.compare_digest(request.headers.get('X-Smotri-Bot-Secret',''),secret):raise HTTPException(403,'Forbidden')
    raw=await request.body()
    if len(raw)>4096:raise HTTPException(413,'Too large')
    try:
        data=json.loads(raw);id=data['id']
        if not isinstance(id,str) or len(id)>64:raise ValueError()
        if data.get('action')=='describe':
            row=bot_auth.describe(id)
            if not row:raise HTTPException(410,'Запрос входа истёк. Начните вход на smotri.cc заново.')
            return {'code':id[:6].upper(),'item':row['item']}
        uid=int(data['uid']);name=str(data.get('name',''))
        if uid<=0:raise ValueError()
    except (ValueError,KeyError,TypeError):raise HTTPException(400,'Invalid request')
    item=bot_auth.confirm(id,uid,name)
    if item is None:raise HTTPException(410,'Запрос входа истёк или уже использован.')
    return {'url':cfg['origin']+'/?login=confirmed&item='+quote(item)+'#packs'}

@app.post('/api/unlock/{item_id}')
async def unlock(item_id: str, request: Request):
    cfg = config(); same_origin(request, cfg); item_by_id(cfg, item_id); s = session(request, cfg)
    if not s.get('uid'): raise HTTPException(401, 'Сначала войдите через Telegram.')
    try:
        async with httpx.AsyncClient(timeout=15) as client:
            r = await client.post('https://api.telegram.org/bot' + cfg['bot_token'] + '/getChatMember', json={'chat_id': cfg['channel_id'], 'user_id': s['uid']})
            result = r.json()
        if not result.get('ok'): raise HTTPException(503, 'Проверка подписки временно недоступна. Попробуйте позже.')
        member = result['result']; status = member.get('status')
        if status not in ('creator', 'administrator', 'member') and not (status == 'restricted' and member.get('is_member')): raise HTTPException(403, 'Подпишитесь на канал и нажмите «Проверить подписку».')
    except (httpx.HTTPError, ValueError): raise HTTPException(503, 'Telegram не отвечает. Попробуйте позже.')
    return grant(s, item_id, cfg)

def grant(s, item_id, cfg):
    # A bounded session grant; private file paths never reach the catalog response.
    grants = list(dict.fromkeys(s.get('grants', []) + [item_id]))[-30:]
    return cookie(JSONResponse({'download': '/download/' + quote(item_id)}), {**s, 'grants': grants, 'exp': int(time.time()) + 3600}, cfg)

@app.post('/api/screenshot/{item_id}')
async def screenshot(item_id: str, request: Request):
    cfg = config(); same_origin(request, cfg); item_by_id(cfg, item_id)
    body = bytearray()
    async for chunk in request.stream():
        body.extend(chunk)
        if len(body) > 8 * 1024 * 1024: raise HTTPException(413, 'Максимум 8 МБ.')
    try:
        with Image.open(io.BytesIO(body)) as im:
            if im.format not in ('PNG', 'JPEG', 'WEBP') or min(im.size) < 32: raise ValueError()
            im.verify()
    except (UnidentifiedImageError, OSError, ValueError, Image.DecompressionBombError, Image.DecompressionBombWarning): raise HTTPException(400, 'Прикрепите изображение PNG, JPG или WebP.')
    # Receipt only, deliberately no claim of semantic verification. Image not retained.
    return grant(session(request, cfg), item_id, cfg)

@app.get('/download/{item_id}')
def download(item_id: str, request: Request):
    cfg = config(); item = item_by_id(cfg, item_id); s = session(request, cfg)
    if item_id not in s.get('grants', []): raise HTTPException(403, 'Сначала получите доступ на странице пака.')
    if item.get('file'):
        path = Path(item['file']).resolve(); root = Path(cfg['pack_root']).resolve()
        if not path.is_relative_to(root) or not path.is_file(): raise HTTPException(404, 'Файл временно недоступен.')
        return FileResponse(path, filename=path.name, media_type='application/octet-stream')
    return RedirectResponse(item['url'], status_code=303)

@app.get('/health')
def health(): return {'ok': True}

@app.api_route('/', methods=['GET', 'HEAD'])
def index(): return FileResponse(ROOT / 'static/index.html')

@app.get('/robots.txt')
def robots():
    return Response('User-agent: *\nAllow: /\nDisallow: /api/\nDisallow: /download/\nSitemap: https://smotri.cc/sitemap.xml\n', media_type='text/plain')

@app.get('/sitemap.xml')
def sitemap():
    return Response('<?xml version="1.0" encoding="UTF-8"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"><url><loc>https://smotri.cc/</loc></url></urlset>', media_type='application/xml')

app.mount('/static', StaticFiles(directory=ROOT / 'static'), name='static')
