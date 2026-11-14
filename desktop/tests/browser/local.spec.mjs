import {test,expect} from '@playwright/test';
import {spawn} from 'node:child_process';
import {mkdtemp,readFile,mkdir} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {resolve,join} from 'node:path';
const normal='desktop browser normal',clean='desktop browser cleanup';
test('real local vault, contacts, delayed Tor identity, groups, visibility watchdog and cleanup',async({page,context},testInfo)=>{
 const root=await mkdtemp(join(tmpdir(),'psychotalk-browser-'));
 const binary=resolve(process.env.PSYCHOTALK_TEST_BINARY||'target/debug/psychotalk');
 const tor=resolve(process.env.PSYCHOTALK_TEST_TOR||'.tools/tor-src/src/app/tor');
 const launchArgs=['--no-open','--data-dir',root];if(process.env.PSYCHOTALK_TEST_USE_BUNDLED_TOR!=='1')launchArgs.push('--tor',tor);
 const processHandle=spawn(binary,launchArgs,{stdio:['ignore','pipe','pipe']});
 let output='',errors='';processHandle.stdout.on('data',b=>output+=b);processHandle.stderr.on('data',b=>errors+=b);
 try{
  await expect.poll(()=>output.match(/http:\/\/127\.0\.0\.1:\d+\/#[a-f0-9]{64}/)?.[0]||'',{timeout:20000,message:'local process starts'}).not.toBe('');
  // Delay only delivery of the real Tor identity to the UI, keeping the real backend intact.
  let holdIdentity=true,actualIdentity='';
  await page.route('**/api/command',async route=>{
   const response=await route.fetch();const data=await response.json();
   if(data.view){if(data.view.own_code)actualIdentity=data.view.own_code;if(holdIdentity)data.view.own_code=null;}
   await route.fulfill({response,json:data});
  });
  const url=output.match(/http:\/\/127\.0\.0\.1:\d+\/#[a-f0-9]{64}/)[0];await page.goto(url);
  await expect(page.getByRole('heading',{name:'Configure seu cofre'})).toBeVisible();
  await page.reload();await expect(page.getByRole('heading',{name:'Configure seu cofre'})).toBeVisible();
  await page.getByLabel('Senha normal',{exact:true}).fill(normal);await page.getByLabel('Confirme a senha normal',{exact:true}).fill(normal);await page.getByLabel('Senha de limpeza local',{exact:true}).fill(clean);await page.getByLabel('Confirme a senha de limpeza',{exact:true}).fill(clean);await page.getByRole('button',{name:'Criar cofre',exact:true}).click();
  await expect(page.getByRole('heading',{name:'Conversas',exact:true})).toBeVisible({timeout:30000});
  await expect(page).toHaveURL(/\/$/);
  const codes=['psychotalk:1:fivcukrkfivcukrkfivcukrkfivcukrkfivcukrkfivcukrkfivjcrid.onion:'+ '2a'.repeat(32),'psychotalk:1:fmvswkzlfmvswkzlfmvswkzlfmvswkzlfmvswkzlfmvswkzlfmvuqiad.onion:'+'2b'.repeat(32)];
  for(const [i,code]of codes.entries()){await page.getByRole('button',{name:'Adicionar contato',exact:true}).click();await page.getByLabel('Tagname local (opcional)').fill(i?'@caio':'@bia');await page.getByLabel('Código público do contato').fill(code);await page.getByRole('button',{name:'Verificar impressão digital'}).click();await page.getByRole('button',{name:'Confirmei com o contato'}).click();await expect(page.getByRole('dialog')).toHaveCount(0);}
  await page.getByLabel('Mensagem de texto').fill('Texto local somente para Caio');await page.getByRole('button',{name:'Enviar',exact:true}).click();await expect(page.locator('.mine .message-body')).toHaveText('Texto local somente para Caio');
  await page.getByRole('navigation',{name:'Conversas'}).getByRole('button',{name:/@bia/}).click();await expect(page.getByText('Texto local somente para Caio')).toHaveCount(0);
  await page.getByRole('button',{name:'Editar tagname',exact:true}).click();await page.getByLabel('Tagname local',{exact:true}).fill('@amiga');await page.getByRole('button',{name:'Salvar tagname'}).click();await expect(page.getByRole('heading',{name:'@amiga'})).toBeVisible();
  await page.getByRole('button',{name:'Novo grupo',exact:true}).click();await page.getByLabel('Nome fixo do grupo').fill('Equipe desktop');await page.getByRole('checkbox').nth(0).check();await expect(page.getByRole('button',{name:'Criar grupo fixo'})).toBeDisabled();await page.getByRole('checkbox').nth(1).check();
  const createGroup=page.getByRole('button',{name:'Criar grupo fixo'});
  await expect(createGroup).toBeDisabled();
  await expect.poll(()=>actualIdentity,{timeout:90000,message:'The real Tor identity becomes available while the form stays open'}).toMatch(/^psychotalk:1:/);
  holdIdentity=false;
  await expect(createGroup).toBeEnabled({timeout:10000});
  await expect(page.getByLabel('Nome fixo do grupo')).toHaveValue('Equipe desktop');
  await expect(page.getByRole('checkbox').nth(0)).toBeChecked();await expect(page.getByRole('checkbox').nth(1)).toBeChecked();
  await createGroup.click();await expect(page.getByRole('heading',{name:'Equipe desktop',exact:true})).toBeVisible();await expect(page.getByRole('button',{name:'Enviar',exact:true})).toBeDisabled();
  await page.getByRole('button',{name:'Apagar minhas mensagens',exact:true}).click();await page.getByRole('button',{name:'Apagar nesta conversa'}).click();await expect(page.getByText(/Exclusão confirmada por 0 de 2/)).toBeVisible();
  await page.getByRole('button',{name:'Participantes',exact:true}).click();await expect(page.getByText('Grupo fixo: 3 participantes')).toBeVisible();await page.getByRole('button',{name:'Copiar convite'}).click();const invite=await page.evaluate(()=>navigator.clipboard.readText());expect(invite).toMatch(/^psychotalk-group:1:/);
  await page.getByRole('button',{name:'Entrar em grupo',exact:true}).click();await page.getByLabel('Convite do grupo').fill(invite);await page.getByRole('button',{name:'Conferir participantes'}).click();await expect(page.getByRole('heading',{name:'Conferir composição do grupo'})).toBeVisible();await page.getByRole('button',{name:'Conferi e aceito entrar'}).click();await expect(page.getByRole('dialog')).toHaveCount(0);await expect(page.getByRole('navigation').getByRole('button')).toHaveCount(3);
  await page.screenshot({path:testInfo.outputPath('desktop.png'),fullPage:true});
  await page.setViewportSize({width:400,height:850});await expect(page.getByRole('button',{name:'Enviar',exact:true})).toBeVisible();expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);await page.screenshot({path:testInfo.outputPath('narrow.png')});
  await page.setViewportSize({width:1440,height:1000});await page.evaluate(()=>document.documentElement.style.zoom='2');expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);await page.getByRole('button',{name:'Enviar',exact:true}).scrollIntoViewIfNeeded();await page.screenshot({path:testInfo.outputPath('zoom200.png')});await page.evaluate(()=>{document.documentElement.style.zoom='1';document.documentElement.dir='rtl';});expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);await page.screenshot({path:testInfo.outputPath('rtl.png')});await page.evaluate(()=>document.documentElement.dir='ltr');
  const beforeReload=await page.evaluate(()=>({token,tab,resume:sessionStorage.getItem(resumeKey)}));
  expect(beforeReload.resume).toMatch(/^[a-f0-9]{64}$/);expect(beforeReload.resume).not.toBe(beforeReload.token);
  await page.reload();await expect(page.getByRole('heading',{name:'Abra seu cofre'})).toBeVisible();
  await expect(page.getByRole('heading',{name:'Conversas',exact:true})).toHaveCount(0);
  await page.reload();await expect(page.getByRole('heading',{name:'Abra seu cofre'})).toBeVisible();
  const stored=await page.evaluate(()=>Object.fromEntries(Object.entries(sessionStorage)));
  expect(Object.keys(stored)).toEqual(['psychotalk.resume.v1']);expect(stored['psychotalk.resume.v1']).not.toBe(beforeReload.resume);
  const address=new URL(page.url());const origin=address.origin;
  const staleHeaders={'Origin':origin,'X-Psychotalk-Token':beforeReload.token,'X-Psychotalk-Tab':beforeReload.tab};
  expect((await context.request.post(origin+'/api/lock',{headers:staleHeaders,data:{}})).status()).toBe(401);
  expect((await context.request.post(origin+'/api/resume',{headers:{...staleHeaders},data:{proof:beforeReload.resume}})).status()).toBe(401);
  await page.getByLabel('Senha',{exact:true}).fill(normal);await page.getByRole('button',{name:'Desbloquear',exact:true}).click();await expect(page.getByRole('heading',{name:'Conversas',exact:true})).toBeVisible();
  await expect(page.getByRole('navigation').getByRole('button')).toHaveCount(3);
  const previous=page;const previousGrant=await page.evaluate(()=>({token,tab}));
  const [duplicate]=await Promise.all([context.waitForEvent('page'),page.evaluate(()=>window.open('/','_blank'))]);
  page=duplicate;
  await expect(page.getByRole('heading',{name:'Abra seu cofre'})).toBeVisible();
  await expect(previous.getByRole('heading',{name:'Abra o Psychotalk novamente'})).toBeVisible();
  await page.getByLabel('Senha',{exact:true}).fill(normal);await page.getByRole('button',{name:'Desbloquear',exact:true}).click();await expect(page.getByRole('heading',{name:'Conversas',exact:true})).toBeVisible();
  for(const path of ['lock','heartbeat','command']){
   expect((await context.request.post(origin+'/api/'+path,{headers:{Origin:origin,'X-Psychotalk-Token':previousGrant.token,'X-Psychotalk-Tab':previousGrant.tab},data:{op:'delete'}})).status()).toBe(401);
  }
  expect((await page.evaluate(()=>api('status'))).unlocked).toBe(true);
  await previous.close();
  // Disable JS without pagehide: the independent native watchdog must revoke the session.
  const cdp=await context.newCDPSession(page);await cdp.send('Emulation.setScriptExecutionDisabled',{value:true});await new Promise(r=>setTimeout(r,3200));await cdp.send('Emulation.setScriptExecutionDisabled',{value:false});await expect(page.getByRole('heading',{name:'Abra seu cofre'})).toBeVisible({timeout:10000});
  let releaseReply,arrivedReply;
  const replyArrived=new Promise(r=>arrivedReply=r),release=new Promise(r=>releaseReply=r);
  const delayUnlock=async route=>{try{const response=await route.fetch();arrivedReply();await release;await route.fulfill({response});}catch{}};
  await page.route('**/api/unlock',delayUnlock);
  await page.getByLabel('Senha',{exact:true}).fill(normal);await page.getByRole('button',{name:'Desbloquear',exact:true}).click();
  await replyArrived;
  try{await page.reload();await expect(page.getByRole('heading',{name:'Abra seu cofre'})).toBeVisible();}
  finally{releaseReply();await page.unroute('**/api/unlock',delayUnlock);}
  expect((await page.evaluate(()=>api('status'))).unlocked).toBe(false);
  await page.getByLabel('Senha',{exact:true}).fill(normal);await page.getByRole('button',{name:'Desbloquear',exact:true}).click();await expect(page.getByRole('heading',{name:'Conversas',exact:true})).toBeVisible({timeout:20000});await page.getByRole('button',{name:'Bloquear',exact:true}).click();await expect(page.getByRole('heading',{name:'Abra seu cofre'})).toBeVisible();
  const beforeCleanup=await page.evaluate(()=>({token,tab,resume:sessionStorage.getItem(resumeKey)}));
  await page.getByLabel('Senha',{exact:true}).fill(clean);await page.getByRole('button',{name:'Desbloquear',exact:true}).click();await expect(page.getByRole('heading',{name:'Abra o Psychotalk novamente'})).toBeVisible({timeout:20000});
  await expect(page.getByText(/Limpeza local concluída/)).toBeVisible();
  await page.evaluate(async()=>{try{await api('heartbeat');}catch{}});
  await expect(page.getByText(/Limpeza local concluída/)).toBeVisible();
  expect(await page.evaluate(()=>sessionStorage.length)).toBe(0);
  expect((await context.request.post(origin+'/api/resume',{headers:{Origin:origin,'X-Psychotalk-Tab':beforeCleanup.tab},data:{proof:beforeCleanup.resume}})).status()).toBe(401);
  expect((await context.request.post(origin+'/api/status',{headers:{Origin:origin,'X-Psychotalk-Token':beforeCleanup.token,'X-Psychotalk-Tab':beforeCleanup.tab},data:{}})).status()).toBe(401);
  await page.reload();await expect(page.getByRole('heading',{name:'Abra o Psychotalk novamente'})).toBeVisible();
  const entries=await import('node:fs/promises').then(fs=>fs.readdir(join(root,'vault')));expect(entries).toEqual(['.owner']);
 }finally{try{await page.evaluate(async password=>{await api('lock');await api('unlock',{password});},clean);}catch{}processHandle.kill('SIGTERM');await Promise.race([new Promise(r=>processHandle.once('exit',r)),new Promise(r=>setTimeout(r,15000))]);if(errors)console.error(errors);}
});

test('native restart revokes resume proof and denied sessionStorage fails closed',async({page,context})=>{
 const root=await mkdtemp(join(tmpdir(),'psychotalk-resume-restart-'));
 const binary=resolve(process.env.PSYCHOTALK_TEST_BINARY||'target/debug/psychotalk');
 const args=['--no-open','--data-dir',root];
 if(process.env.PSYCHOTALK_TEST_USE_BUNDLED_TOR!=='1')args.push('--tor',resolve(process.env.PSYCHOTALK_TEST_TOR||'.tools/tor-src/src/app/tor'));
 let child;
 async function start(){
  child=spawn(binary,args,{stdio:['ignore','pipe','pipe']});let output='';child.stdout.on('data',b=>output+=b);
  await expect.poll(()=>output.match(/http:\/\/127\.0\.0\.1:\d+\/#[a-f0-9]{64}/)?.[0]||'',{timeout:20000}).not.toBe('');
  return output.match(/http:\/\/127\.0\.0\.1:\d+\/#[a-f0-9]{64}/)[0];
 }
 async function stop(){if(child&&child.exitCode===null){const done=new Promise(r=>child.once('exit',r));child.kill('SIGTERM');await done;}}
 try{
  await page.goto(await start());await expect(page.getByRole('heading',{name:'Configure seu cofre'})).toBeVisible();
  const proof=await page.evaluate(()=>sessionStorage.getItem(resumeKey));expect(proof).toMatch(/^[a-f0-9]{64}$/);
  await stop();const url=await start(),origin=new URL(url).origin;
  const response=await context.request.post(origin+'/api/resume',{headers:{Origin:origin,'X-Psychotalk-Tab':'e'.repeat(64)},data:{proof}});expect(response.status()).toBe(401);
  await page.addInitScript(()=>Object.defineProperty(window,'sessionStorage',{get(){throw new DOMException('Storage unavailable','SecurityError');}}));
  await page.goto(url);await expect(page.getByRole('heading',{name:'Configure seu cofre'})).toBeVisible();
  await expect(page.getByText(/Seu navegador bloqueou a retomada/)).toBeVisible();
  await page.reload();await expect(page.getByRole('heading',{name:'Abra o Psychotalk novamente'})).toBeVisible();
 }finally{await stop();}
});
