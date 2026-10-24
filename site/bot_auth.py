"""One-time browser-bound login challenges, shared by the website and existing Telegram bot."""
import hashlib
import hmac
import os
import secrets
import sqlite3
import time
from pathlib import Path

DB = Path(os.environ.get('SMOTRI_AUTH_DB', '/var/lib/smotri/auth.sqlite'))

def connect():
    db=sqlite3.connect(DB,timeout=5)
    db.row_factory=sqlite3.Row
    db.execute('CREATE TABLE IF NOT EXISTS challenges (id TEXT PRIMARY KEY, browser_hash TEXT NOT NULL, item TEXT NOT NULL, expires INTEGER NOT NULL, uid INTEGER, name TEXT, consumed INTEGER NOT NULL DEFAULT 0)')
    db.execute('DELETE FROM challenges WHERE expires < ?', (int(time.time())-60,))
    db.commit()
    return db

def create(item):
    id=secrets.token_urlsafe(24);secret=secrets.token_urlsafe(32)
    with connect() as db:
        db.execute('INSERT INTO challenges (id,browser_hash,item,expires) VALUES (?,?,?,?)',(id,hashlib.sha256(secret.encode()).hexdigest(),item,int(time.time())+600))
    return id,secret

def describe(id):
    with connect() as db:row=db.execute('SELECT item,expires,consumed FROM challenges WHERE id=?',(id,)).fetchone()
    if not row or row['expires']<time.time() or row['consumed']:return None
    return dict(row)

def confirm(id,uid,name):
    with connect() as db:
        db.execute('BEGIN IMMEDIATE')
        row=db.execute('SELECT * FROM challenges WHERE id=?',(id,)).fetchone()
        if not row or row['expires']<time.time() or row['consumed']:return None
        if row['uid'] is not None and row['uid']!=uid:return None
        db.execute('UPDATE challenges SET uid=?,name=? WHERE id=?',(uid,name[:100],id))
        return row['item']

def consume(id,secret):
    with connect() as db:
        db.execute('BEGIN IMMEDIATE')
        row=db.execute('SELECT * FROM challenges WHERE id=?',(id,)).fetchone()
        if not row or row['expires']<time.time() or row['consumed']:return {'state':'expired'}
        if not hmac.compare_digest(row['browser_hash'],hashlib.sha256(secret.encode()).hexdigest()):return {'state':'expired'}
        if row['uid'] is None:return {'state':'pending'}
        db.execute('UPDATE challenges SET consumed=1 WHERE id=?',(id,))
        return {'state':'confirmed','uid':row['uid'],'name':row['name'],'item':row['item']}
