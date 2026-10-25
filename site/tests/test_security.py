import hashlib,hmac,io,json,tempfile,time,unittest
from pathlib import Path
from unittest.mock import patch
from fastapi.testclient import TestClient
from PIL import Image
import server
import bot_auth

class SecurityTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();root=Path(self.tmp.name)
        (root/'test.prfpset').write_bytes(b'original preset')
        self.cfg={'origin':'https://smotri.cc','session_key':'test-session-secret','bot_token':'test-token','bot_username':'TestBot','channel_id':'-100123','channel_url':'https://t.me/test','pack_root':str(root),'catalog':[{'id':'pack','name':'Test','file':str(root/'test.prfpset'),'category':'video_packs'}],'releases':[{'id':'mac-arm','name':'Smotri macOS','file':str(root/'test.prfpset')}]}
        self.dbpatch=patch.object(bot_auth,'DB',root/'auth.sqlite');self.dbpatch.start()
        self.cfg['bot_login_secret']='test-bridge-secret'
        self.patcher=patch.object(server,'config',return_value=self.cfg);self.patcher.start()
        self.client=TestClient(server.app,base_url='https://smotri.cc');self.headers={'Origin':'https://smotri.cc'}
    def tearDown(self):self.patcher.stop();self.dbpatch.stop();self.tmp.cleanup()
    def test_no_secrets_or_paths_in_catalog(self):
        r=self.client.get('/api/catalog');self.assertEqual(r.status_code,200)
        for value in ('test-token','test-session-secret',self.tmp.name,'test.prfpset'):self.assertNotIn(value,r.text)
        self.assertNotIn('file',r.json()['releases'][0])
    def test_preview_metadata_is_public_but_download_stays_gated(self):
        self.cfg['releases'][0].update(version='0.12.0-preview.20261025', channel='preview')
        release=self.client.get('/api/catalog').json()['releases'][0]
        self.assertEqual(release['channel'],'preview')
        self.assertEqual(release['version'],'0.12.0-preview.20261025')
        self.assertNotIn('url',release)
        self.assertEqual(self.client.get('/download/'+release['id']).status_code,403)
    def test_packs_and_editor_require_grant(self):
        for id in ('pack','mac-arm'):self.assertEqual(self.client.get('/download/'+id).status_code,403)
    def test_tampered_and_expired_sessions(self):
        token=server.sign({'exp':int(time.time())+100,'grants':['pack']},'wrong-key')
        self.assertIsNone(server.verify(token,self.cfg['session_key']))
        token=server.sign({'exp':1},self.cfg['session_key']);self.assertIsNone(server.verify(token,self.cfg['session_key']))
    def test_forged_telegram_and_csrf_rejected(self):
        self.assertEqual(self.client.post('/api/auth/bot',json={'id':'fake','uid':1}).status_code,403)
        self.assertEqual(self.client.post('/api/unlock/pack',headers={'Origin':'https://evil.example'}).status_code,403)
        self.assertEqual(self.client.post('/api/unlock/pack',headers=self.headers).status_code,401)
    def test_bot_login_is_browser_bound_and_single_use(self):
        r=self.client.post('/api/auth/start',json={'item':'pack'},headers=self.headers)
        self.assertEqual(r.status_code,200);id=r.json()['url'].split('smotri_')[-1]
        self.assertEqual(self.client.get('/api/auth/poll').json()['state'],'pending')
        other=TestClient(server.app,base_url='https://smotri.cc')
        self.assertEqual(other.get('/api/auth/poll').json()['state'],'missing')
        r=self.client.post('/api/auth/bot',json={'id':id,'uid':123,'name':'Test'},headers={'X-Smotri-Bot-Secret':'test-bridge-secret'})
        self.assertEqual(r.status_code,200);self.assertIn('item=pack',r.json()['url'])
        self.assertEqual(self.client.get('/api/auth/poll').json()['state'],'confirmed')
        self.assertTrue(self.client.get('/api/session').json()['authenticated'])
        self.assertFalse(other.get('/api/session').json()['authenticated'])
        self.assertEqual(self.client.post('/api/auth/bot',json={'id':id,'uid':456},headers={'X-Smotri-Bot-Secret':'test-bridge-secret'}).status_code,410)
        self.assertEqual(self.client.get('/download/pack').status_code,403)

    def test_telegram_signature_and_freshness(self):
        data={'id':123,'auth_date':1000,'first_name':'Test'}
        msg='\n'.join(f'{k}={data[k]}' for k in sorted(data));data['hash']=hmac.new(hashlib.sha256(b'test-token').digest(),msg.encode(),hashlib.sha256).hexdigest()
        self.assertTrue(server.telegram_valid(data,'test-token',1001));self.assertFalse(server.telegram_valid(data,'test-token',1400));data['id']=456;self.assertFalse(server.telegram_valid(data,'test-token',1001))
    def test_screenshot_receipt_unlocks_only_selected_item(self):
        out=io.BytesIO();Image.new('RGB',(64,64),'black').save(out,'PNG')
        r=self.client.post('/api/screenshot/pack',content=out.getvalue(),headers=self.headers);self.assertEqual(r.status_code,200)
        self.assertIn('HttpOnly',r.headers['set-cookie']);self.assertIn('Secure',r.headers['set-cookie'])
        self.assertEqual(self.client.get('/download/pack').content,b'original preset');self.assertEqual(self.client.get('/download/mac-arm').status_code,403)
    def test_invalid_screenshot_and_path_escape(self):
        self.assertEqual(self.client.post('/api/screenshot/pack',content=b'<script>evil</script>',headers=self.headers).status_code,400)
        self.cfg['catalog'][0]['file']='/etc/passwd'
        self.client.cookies.set('smotri_session',server.sign({'exp':int(time.time())+100,'grants':['pack']},self.cfg['session_key']))
        self.assertEqual(self.client.get('/download/pack').status_code,404)


    def test_public_sections_and_social_links(self):
        for path in ('/', '/packs', '/plugins', '/guides', '/links'):
            r=self.client.get(path)
            self.assertEqual(r.status_code, 200)
            self.assertIn('https://smotri.cc'+path, r.text)
            self.assertNotIn('https://vk.com', r.text)
            self.assertNotIn('t.me/acekillthatmp4', r.text)
            self.assertIn('https://www.instagram.com/aktmp4', r.text)
            self.assertIn('https://www.tiktok.com/@maybeaivideo', r.text)
        self.assertEqual(self.client.get('/missing-page').status_code, 404)
        self.assertIn('/packs</loc>', self.client.get('/sitemap.xml').text)
        self.assertIn('Sitemap:', self.client.get('/robots.txt').text)

if __name__=='__main__':unittest.main()
