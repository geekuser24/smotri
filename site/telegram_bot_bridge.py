"""Small integration for the existing python-telegram-bot application.
Copy beside its entry point; private configuration is provisioned server-side.
"""
import asyncio
import json
from pathlib import Path
import urllib.request
import urllib.error
from telegram import InlineKeyboardButton, InlineKeyboardMarkup

CONFIG=Path(__file__).with_name('smotri-login-secret.json')

def request_sync(payload):
    cfg=json.loads(CONFIG.read_text())
    req=urllib.request.Request(cfg.get('endpoint','https://smotri.cc/api/auth/bot'),data=json.dumps(payload).encode(),headers={'Content-Type':'application/json','Host':'smotri.cc','X-Smotri-Bot-Secret':cfg['secret']})
    try:
        with urllib.request.urlopen(req,timeout=20) as r:return json.load(r)
    except urllib.error.HTTPError as e:
        try:message=json.load(e).get('detail','Вход временно недоступен.')
        except (ValueError,AttributeError):message='Вход временно недоступен.'
        raise ValueError(message) from None
    except (OSError,ValueError):raise ValueError('Сайт временно недоступен. Попробуйте ещё раз.') from None

async def start(update,context):
    if not context.args or not context.args[0].startswith('smotri_'):return False
    id=context.args[0][7:]
    try:
        result=await asyncio.to_thread(request_sync,{'action':'describe','id':id})
        await update.message.reply_text('Вход на smotri.cc\n\nКод запроса: '+result['code']+'\n\nПодтверди, только если ты сам начал вход на smotri.cc и код совпадает с кодом на сайте. Запрос действует 10 минут.',reply_markup=InlineKeyboardMarkup([[InlineKeyboardButton('Подтвердить вход',callback_data='smotri_login:'+id)]]))
    except ValueError as e:await update.message.reply_text(str(e))
    return True

async def confirm(update,context):
    query=update.callback_query
    await query.answer()
    try:
        user=query.from_user
        result=await asyncio.to_thread(request_sync,{'action':'confirm','id':query.data.split(':',1)[1],'uid':user.id,'name':user.first_name or ''})
        await query.edit_message_text('Вход подтверждён ✅\n\nИсходная вкладка сайта уже получает подтверждение. Вернись к ней или нажми кнопку ниже. Если откроется другой браузер, вернись в тот, где начинал вход.',reply_markup=InlineKeyboardMarkup([[InlineKeyboardButton('Вернуться на сайт',url=result['url'])]]))
    except ValueError as e:await query.edit_message_text(str(e))
