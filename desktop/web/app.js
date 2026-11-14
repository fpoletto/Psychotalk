'use strict';
const root = document.querySelector('#app');
let bootstrap = location.hash.slice(1),token='',authorized=false,resumeAvailable=true;
const resumeKey='psychotalk.resume.v1';
history.replaceState(null, '', '/');
const tab = [...crypto.getRandomValues(new Uint8Array(32))].map(n=>n.toString(16).padStart(2,'0')).join('');
let generation=0, unlocked=false, selected=null, snapshot=null, status={}, polling=false, busy=false, pendingLock=Promise.resolve();
let dialog=null, renderedEpoch=-1;
function el(tag, attrs={}, ...children) {
  const node=document.createElement(tag);
  for(const [key,value] of Object.entries(attrs)) {
    if(value===undefined||value===null)continue;
    if(key==='class') node.className=value;
    else if(key.startsWith('on')) node.addEventListener(key.slice(2).toLowerCase(), value);
    else if(key==='text') node.textContent=value;
    else if(key in node) node[key]=value;
    else node.setAttribute(key,value);
  }
  for(const child of children.flat(Infinity)) if(child!=null) node.append(child instanceof Node ? child : document.createTextNode(String(child)));
  return node;
}
const button=(text,action,kind='secondary')=>el('button',{type:'button',class:kind,onclick:action},text);
const text=(value,kind='')=>el('p',{class:kind},value);
function field(label,type='text',value='') {
  const input=el(type==='textarea'?'textarea':'input',{type:type==='textarea'?undefined:type,value,autocomplete:'off',spellcheck:false});
  return {input,node:el('label',{class:'field'},el('span',{},label),input)};
}
function dismiss(){if(dialog){dialog.close();dialog.remove();dialog=null;}}
function modal(title,...children){dismiss();dialog=el('dialog',{},el('header',{class:'modal-heading'},el('h2',{},title),button('Fechar',dismiss,'quiet')),el('div',{class:'modal-body'},children));dialog.addEventListener('close',()=>{if(dialog&&!dialog.open){dialog.remove();dialog=null;}});document.body.append(dialog);dialog.showModal();}
function notice(message){const area=document.querySelector('#notice');if(area)area.textContent=message;}
async function api(path,body={}) {
  const response=await fetch('/api/'+path,{method:'POST',cache:'no-store',credentials:'omit',headers:{'Content-Type':'application/json','X-Psychotalk-Token':token,'X-Psychotalk-Tab':tab},body:JSON.stringify(body)});
  const result=await response.json();
  if(!response.ok){if(result.reauthorize&&authorized)authorizationLost();throw new Error(result.error||'Operação recusada.');}
  return result;
}
function forgetResume(){try{sessionStorage.removeItem(resumeKey);}catch{}}
function authorizationLost(message='Esta janela perdeu a autorização. Abra o Psychotalk novamente pelo comando abaixo.'){
  authorized=false;token='';forgetResume();lockScreen();
  root.replaceChildren(el('section',{class:'lock-card'},brand(),el('h1',{},'Abra o Psychotalk novamente'),text(message),el('pre',{},'~/.local/bin/psychotalk')));
}
async function authorizeWindow(){
  let proof=bootstrap,kind='authorize';bootstrap='';
  if(!proof){try{proof=sessionStorage.getItem(resumeKey)||'';}catch{}kind='resume';}
  if(!/^[a-f0-9]{64}$/.test(proof)){authorizationLost();return;}
  root.replaceChildren(el('section',{class:'lock-card'},brand(),text('Autorizando esta janela…')));
  try{
    const grant=await api(kind,{proof});token=grant.token;authorized=true;
    try{sessionStorage.setItem(resumeKey,grant.resume);}catch{resumeAvailable=false;forgetResume();}
    grant.token='';grant.resume='';await refreshStatus();
  }catch{authorizationLost('A autorização expirou ou outra janela assumiu o controle. Abra o Psychotalk novamente.');}
  finally{proof='';}
}
function brand(){return el('div',{class:'brand'},el('span',{class:'brand-mark','aria-hidden':'true'},'P'),el('div',{},el('strong',{},'Psychotalk'),el('small',{},'Seu cofre. Sua conversa.')));}
function lockScreen(message='') {
  unlocked=false;generation++;selected=null;snapshot=null;renderedEpoch=-1;dismiss();busy=false;
  renderLock(message);
}
function requestLock(message='Sessão bloqueada.') {
  if(!authorized)return;
  lockScreen(message);
  pendingLock=api('lock').catch(()=>{}).finally(()=>{if(!unlocked)refreshStatus();});
}
async function refreshStatus(){if(!authorized)return;const g=generation;try{const data=await api('status');if(!unlocked&&g===generation){const changed=status.configured!==data.configured;status=data;if(changed||!document.querySelector('.lock-form'))renderLock();}}catch(e){if(authorized&&!unlocked&&g===generation&&!busy)renderLock(e.message);}}
function renderLock(message='') {
  if(!message&&!resumeAvailable)message='Seu navegador bloqueou a retomada. Recarregar exigirá abrir o aplicativo novamente.';
  const setup=!status.configured;
  const normal=field(setup?'Senha normal':'Senha','password');
  const confirm=field('Confirme a senha normal','password');
  const cleanup=field('Senha de limpeza local','password');
  const repeat=field('Confirme a senha de limpeza','password');
  const submit=el('button',{type:'submit',class:'primary'},setup?'Criar cofre':'Desbloquear');
  const error=text(message,'form-error');error.setAttribute('role','status');
  const form=el('form',{class:'lock-form'},normal.node,setup?[confirm.node,cleanup.node,repeat.node]:[],error,submit);
  form.addEventListener('submit',async event=>{
    event.preventDefault();if(busy)return;
    const fields=setup?[normal,confirm,cleanup,repeat]:[normal];
    if(fields.some(f=>!f.input.value||new TextEncoder().encode(f.input.value).length>1024)){error.textContent='Preencha as senhas com até 1024 bytes cada.';return;}
    if(setup&&(normal.input.value!==confirm.input.value||cleanup.input.value!==repeat.input.value||normal.input.value===cleanup.input.value)){error.textContent='Confirme as senhas e escolha uma senha de limpeza diferente.';return;}
    const body={password:normal.input.value};if(setup)body.cleanup=cleanup.input.value;
    fields.forEach(f=>f.input.value='');busy=true;submit.disabled=true;submit.textContent='Validando cofre…';const g=generation;
    try{await pendingLock;const result=await api('unlock',body);if(g!==generation)return;
      status=result;if(result.cleaned){authorizationLost('Limpeza local concluída. Abra o Psychotalk novamente para configurar um novo cofre.');return;}
      unlocked=true;generation++;renderShell();await api('heartbeat');await poll();
    }catch(e){if(g===generation)error.textContent=e.message;}finally{body.password='';body.cleanup='';busy=false;submit.disabled=false;submit.textContent=setup?'Criar cofre':'Desbloquear';}
  });
  root.replaceChildren(el('div',{class:'lock-layout'},el('section',{class:'lock-intro'},brand(),el('h1',{},'Conversas privadas,\nno seu computador.'),text('Tor e autenticação mútua entre participantes. Seu histórico fica no cofre local.'),el('span',{class:'eyebrow'},'DESKTOP EXPERIMENTAL · '+(status.version||'0.4.2'))),el('section',{class:'lock-card'},el('span',{class:'eyebrow'},setup?'PRIMEIRO ACESSO':'BEM-VINDO DE VOLTA'),el('h2',{},setup?'Configure seu cofre':'Abra seu cofre'),text(setup?'Guarde as duas senhas: não há recuperação. A senha de limpeza apaga todo o cofre local e não apaga cópias nos contatos.':'Use sua senha para liberar o histórico e iniciar a conexão Tor.','muted'),form,text(status.keyring||'Proteção pelo chaveiro do sistema','keyring'))));
}
function renderShell(){
  root.replaceChildren(el('div',{class:'workspace'},el('aside',{class:'sidebar'},el('header',{class:'sidebar-heading'},brand(),el('div',{class:'status-line',id:'network'},'Conectando ao Tor')),el('div',{class:'sidebar-actions'},button('Adicionar contato',addContact,'primary'),button('Novo grupo',newGroup),button('Entrar em grupo',importGroup)),el('div',{class:'list-heading'},el('h2',{},'Conversas'),button('Meu código',ownCode,'quiet')),el('nav',{id:'conversations','aria-label':'Conversas'}),el('footer',{class:'sidebar-footer'},text('Cofre local · '+(status.keyring||''),'muted'),button('Bloquear',()=>requestLock()))),el('section',{class:'chat-pane',id:'chat','aria-label':'Conversa'}),el('div',{id:'notice',role:'status','aria-live':'polite'})));
  renderChatStructure();
}
function renderChatStructure(){
 const pane=document.querySelector('#chat');if(!pane)return;
 root.dataset.chat=Boolean(selected);
 if(!selected){pane.replaceChildren(el('div',{class:'empty-state'},el('span',{class:'empty-mark','aria-hidden':'true'},'P'),el('h1',{},'Espaço para conversar.'),text('Escolha uma conversa ou adicione um contato para começar.','muted'),text('Troquem os códigos por um canal confiável e confirmem as impressões digitais.','muted')));return;}
 const input=el('textarea',{id:'draft',rows:2,placeholder:'Escreva uma mensagem…','aria-label':'Mensagem de texto',maxlength:4096});
 const send=el('button',{type:'submit',id:'send',class:'primary'},'Enviar');
 const composer=el('form',{class:'composer'},input,send);
 composer.addEventListener('submit',async e=>{e.preventDefault();const body=input.value;if(!body||send.disabled)return;if(new TextEncoder().encode(body).length>4096){notice('A mensagem pode ter até 4096 bytes UTF-8.');return;}
 send.disabled=true;try{await mutate('send',{text:body});if(document.querySelector('#draft')===input)input.value='';}catch(e){notice(e.message);}finally{updateReady();}});
 input.addEventListener('keydown',e=>{if(e.key==='Enter'&&!e.shiftKey&&!e.isComposing){e.preventDefault();composer.requestSubmit();}});
 pane.replaceChildren(el('header',{class:'chat-header'},button('← Conversas',()=>choose(null),'quiet mobile-back'),el('div',{class:'chat-title'},el('h1',{id:'chat-name'},'Abrindo conversa…'),text('','muted').cloneNode()),el('div',{class:'chat-actions'},button('Participantes',members),button('Editar tagname',renameDirect),button('Meu código',ownCode,'quiet'),button('Apagar minhas mensagens',deleteMine,'danger'))),el('div',{class:'conversation-state',id:'conversation-state'}),el('div',{class:'messages',id:'messages','aria-label':'Mensagens'}),composer);
}
function active(){return snapshot?.view?.active?.id===selected?snapshot.view.active:null;}
function updateReady(){const chat=active();const ready=chat&&(!chat.group||chat.group.ready===chat.group.total);const send=document.querySelector('#send'),draft=document.querySelector('#draft');if(send)send.disabled=!ready;if(draft)draft.disabled=!ready;}
function renderSnapshot(){
 if(!snapshot||!unlocked)return;
 dialog?.dispatchEvent(new Event('snapshot'));
 document.querySelector('#network').textContent=snapshot.network||'Conectando ao Tor';
 const list=document.querySelector('#conversations');
 const conversations=snapshot.view.conversations;
 for(const [i,c] of conversations.entries()){
   let line=[...list.children].find(n=>n.dataset.id===c.id);
   if(!line){line=el('button',{type:'button',onclick:()=>choose(c.id)},el('span',{class:'avatar','aria-hidden':'true'}),el('span',{class:'conversation-info'}));line.dataset.id=c.id;}
   line.className='conversation'+(c.id===selected?' selected':'');line.setAttribute('aria-current',c.id===selected?'page':'false');
   line.firstChild.textContent=c.kind==='group'?'G':c.tagname.slice(0,1).toUpperCase();
   line.lastChild.replaceChildren(el('strong',{},c.tagname),el('small',{},c.kind==='group'?'Grupo fixo · via Tor':snapshot.connections?.[c.id]||'Offline'),...(c.pending?[el('small',{class:'pending'},`${c.pending} entrega(s) pendente(s)`)]:[]),...(c.deletion_pending?[el('small',{class:'pending'},'Exclusão aguardando confirmação')]:[]));
   if(list.children[i]!==line)list.insertBefore(line,list.children[i]||null);
 }
 while(list.children.length>conversations.length)list.lastChild.remove();
 if(!conversations.length)list.replaceChildren(text('Nenhuma conversa ainda. Adicione seu primeiro contato.','list-empty'));
 const chat=active();if(!chat){updateReady();return;}
 document.querySelector('#chat-name').textContent=chat.tagname;
 const buttons=document.querySelector('.chat-actions').children;buttons[0].hidden=!chat.group;buttons[1].hidden=Boolean(chat.group);
 const state=document.querySelector('#conversation-state');state.replaceChildren();
 if(chat.group){state.append(text(`${chat.group.ready} de ${chat.group.total} participantes prontos`,'muted'));if(chat.group.ready!==chat.group.total)state.append(text('Aguardando aceitação e sincronização de todos. Mantenham os aplicativos abertos e desbloqueados.','pending'));if(chat.group.deletion)state.append(text(`Exclusão confirmada por ${chat.group.deletion.confirmed} de ${chat.group.deletion.total} destinatários. Veja os detalhes em Participantes.`,'pending'));}
 else {state.append(text(snapshot.connections?.[selected]||'Offline','muted'));if(chat.deletion!==null)state.append(text(chat.deletion?'Exclusão confirmada pelo aplicativo do contato':'Exclusão pendente no outro aparelho','pending'));}
 const messages=document.querySelector('#messages');const bottom=messages.scrollHeight-messages.clientHeight-messages.scrollTop<100;
 // Replace visible bodies before acknowledging the core's redaction epoch. No preview cache.
 const receipt=m=>m.mine?(m.recipient_count>1?`Entregue a ${m.delivered_count} de ${m.recipient_count}`:m.delivered?'Entregue':'Pendente'):'';
 const rows=[...messages.querySelectorAll('.message-row')];
 const unchanged=renderedEpoch===snapshot.epoch&&rows.length===chat.messages.length&&chat.messages.every((m,i)=>rows[i].classList.contains('mine')===m.mine&&rows[i].querySelector('.message-body').textContent===m.text&&rows[i].querySelector('.message-author').textContent===(m.mine?'Você':m.author)&&(rows[i].querySelector('.message-receipt')?.textContent||'')===receipt(m));
 if(!unchanged||(!rows.length&&!messages.children.length)){
   messages.replaceChildren();
   for(const m of chat.messages){const info=receipt(m);messages.append(el('article',{class:'message-row '+(m.mine?'mine':'theirs')},el('div',{class:'bubble'},el('span',{class:'message-author'},m.mine?'Você':m.author),text(m.text,'message-body'),info?el('small',{class:'message-receipt'},info):null)));}
   if(!chat.messages.length)messages.append(el('div',{class:'conversation-empty'},text('A conversa começa aqui.','muted')));
   if(bottom)messages.scrollTop=messages.scrollHeight;
 }
 renderedEpoch=snapshot.epoch;
 updateReady();
}
async function choose(id){selected=id;snapshot=null;renderedEpoch=-1;generation++;dismiss();notice('');renderChatStructure();await poll(true);}
async function poll(force=false){if(!unlocked||document.hidden||polling&&!force)return;polling=true;const g=generation,cid=selected;try{const result=await api('command',{op:'view',conversation:cid||''});if(g!==generation||!unlocked)return;snapshot=result;renderSnapshot();requestAnimationFrame(()=>{if(g===generation&&unlocked&&!document.hidden)api('command',{op:'displayed',epoch:result.epoch}).catch(()=>{});});}catch(e){if(g===generation&&unlocked)requestLock('Sessão encerrada. Desbloqueie novamente.');}finally{polling=false;}}
async function mutate(op,fields={}){const g=generation,cid=selected;const result=await api('command',{op,conversation:cid||'',...fields});if(g!==generation||!unlocked)return result;
 if(['pair','create_group','import_group'].includes(op)){selected=result.view.active.id;generation++;renderChatStructure();}
 snapshot=result;renderSnapshot();return result;}
function ownCode(){const code=snapshot?.view?.own_code;modal('Meu código público',text('Troque pelo canal confiável e confirme a impressão digital completa.'),el('pre',{},code||'Preparando o endereço onion…'),button('Copiar código',()=>copy(code),'primary'));}
async function copy(value){if(!value)return;try{await navigator.clipboard.writeText(value);notice('Código público copiado.');dismiss();}catch{notice('Não foi possível copiar. Selecione e copie o código exibido.');}}
function addContact(){const name=field('Tagname local (opcional)'),code=field('Código público do contato','textarea');name.input.maxLength=128;code.input.maxLength=180;
 modal('Adicionar contato',text('O tagname fica somente no seu cofre.'),name.node,code.node,button('Verificar impressão digital',()=>{const c=code.input.value.trim(),n=name.input.value.trim();if(!/^psychotalk:1:[a-z2-7]{56}\.onion:[a-fA-F0-9]{64}$/.test(c)){notice('Confira o código público completo.');return;}modal('Confirmar impressão digital',text('Confirme com o contato pelo canal confiável. Ambos precisam confirmar no próprio aparelho.'),el('pre',{},c.split(':').at(-1)),button('Confirmei com o contato',()=>finish('pair',{code:c,tagname:n}),'primary'));},'primary'));}
async function finish(op,fields){try{await mutate(op,fields);dismiss();notice('');}catch(e){notice(e.message);}}
function renameDirect(){const chat=active();if(chat)rename(chat.tagname,null);}
function rename(name,pin){const value=field('Tagname local', 'text',name);modal('Editar tagname',value.node,text('Visível somente neste cofre. Deixe vazio para usar a identificação abreviada.'),button('Salvar tagname',()=>{const name=value.input.value.trim();if([...name].length>64){notice('Use até 64 caracteres.');return;}finish(pin?'rename_member':'rename',{tagname:name,...(pin?{pin}:{})});},'primary'));}
function deleteMine(){if(!active())return;modal('Apagar suas mensagens desta conversa?',text('Remove seus textos enviados até agora nesta conversa. Cada destinatário confirma separadamente após aplicar a exclusão. Quem estiver offline permanece pendente.'),text('Capturas, cópias externas e clientes modificados ficam fora desse mecanismo.','muted'),button('Apagar nesta conversa',()=>finish('delete',{}),'danger'));}
function newGroup(){const contacts=snapshot?.view?.conversations.filter(c=>c.kind==='direct')||[];const name=field('Nome fixo do grupo');const choices=[];const chosen=new Set();const submit=button('Criar grupo fixo',()=>finish('create_group',{tagname:name.input.value,members:[...chosen]}),'primary');const update=()=>submit.disabled=chosen.size<2||chosen.size>4||!name.input.value.trim()||!snapshot?.view?.own_code;name.input.addEventListener('input',update);
 for(const contact of contacts){const check=el('input',{type:'checkbox',onchange:()=>{if(check.checked){if(chosen.size===4){check.checked=false;return;}chosen.add(contact.id);}else chosen.delete(contact.id);update();}});choices.push(el('label',{class:'member-choice'},check,el('span',{},el('strong',{},contact.tagname),el('small',{class:'fingerprint'},contact.fingerprint.slice(0,16)+'…'))));}
 update();modal('Novo grupo',text('Escolha de 2 a 4 contatos. Com você, serão de 3 a 5 participantes fixos. Nome e composição não podem mudar.'),name.node,contacts.length?choices:text('Adicione e confirme pelo menos dois contatos individuais antes de criar um grupo.','pending'),text('O convite revela aos participantes os endereços onion e as impressões digitais de todos.'),submit);dialog.addEventListener('snapshot',update);}
function importGroup(){const code=field('Convite do grupo','textarea');code.input.maxLength=8192;code.input.rows=5;modal('Entrar em grupo',text('Seu código público precisa estar na composição. Cole o convite recebido por um canal confiável.'),code.node,button('Conferir participantes',async()=>{const invitation=code.input.value.trim();try{const {preview}=await api('command',{op:'preview_group',code:invitation});if(!unlocked)return;modal('Conferir composição do grupo',el('h3',{},preview.name),text('Confirme a impressão do grupo e cada participante pelo canal confiável antes de aceitar.'),el('pre',{},preview.id),...preview.members.map(m=>el('pre',{},m.code)),button('Conferi e aceito entrar',()=>finish('import_group',{code:invitation}),'primary'));}catch(e){notice(e.message);}},'primary'));}
function members(){const group=active()?.group;if(!group)return;const own=snapshot.view.own_code;const rows=group.members.map(m=>{const receipt=group.deletion?.recipients.find(r=>r.fingerprint===m.fingerprint);return el('section',{class:'member-detail'},el('h3',{},m.name),text(m.ready?'Pronto':m.joined?'Aceitou · sincronizando':'Aguardando aceitação','muted'),el('pre',{},m.code),receipt?text(receipt.confirmed?'Exclusão confirmada por este aplicativo':'Exclusão pendente neste destinatário','pending'):null,m.code!==own?button('Editar tagname de '+m.name,()=>rename(m.name,m.fingerprint)):null);});modal('Participantes e convite',text(`Grupo fixo: ${group.total} participantes`),text('A prontidão indica formação concluída; não significa presença online atual.','muted'),el('pre',{},group.fingerprint),rows,text('O convite contém os códigos públicos de todos. Compartilhe somente pelo canal confiável.'),button('Copiar convite',()=>copy(group.invitation),'primary'));}
document.addEventListener('visibilitychange',()=>{if(document.hidden){if(unlocked||busy)requestLock();}else if(authorized&&!unlocked)refreshStatus();});
window.addEventListener('pagehide',()=>{if(authorized&&(unlocked||busy)){lockScreen();fetch('/api/lock',{method:'POST',keepalive:true,headers:{'X-Psychotalk-Token':token,'X-Psychotalk-Tab':tab}}).catch(()=>{});}});
setInterval(()=>{if(unlocked&&!document.hidden)api('heartbeat').catch(()=>requestLock('Sessão encerrada. Desbloqueie novamente.'));},300);
setInterval(()=>poll(),500);
authorizeWindow();
