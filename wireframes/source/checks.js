// Evaluate the saved FIG, after serialization has resolved geometry.
const overflow=figma.root.findAll(n=>n.parent && !['PAGE','CANVAS','DOCUMENT'].includes(n.parent.type) && !['VECTOR','INSTANCE'].includes(n.type) && !['INSTANCE','COMPONENT'].includes(n.parent.type) && (n.x < -1 || n.y < -1 || n.x+n.width>n.parent.width+1 || n.y+n.height>n.parent.height+1)).map(n=>({name:n.name,parent:n.parent.name,x:n.x,y:n.y,w:n.width,h:n.height,pw:n.parent.width,ph:n.parent.height}));
const tiny=figma.root.findAll(n=>n.type==='TEXT'&&n.characters.length>4&&n.width<28).map(n=>({name:n.name,width:n.width}));
const errors=[], measurements={};
if(figma.root.findAll(n=>/pass2p/i.test(n.name)||(n.type==='TEXT'&&/pass2p/i.test(n.characters))).length)errors.push('branding: obsolete product name');
if(!figma.root.findAll(n=>n.type==='TEXT'&&n.characters==='Taypeer').length)errors.push('branding: product name missing');
const boards=figma.root.children.flatMap(p=>p.children);
const board=name=>boards.find(n=>n.name===name);
function pos(n,root){let x=0,y=0;while(n&&n.id!==root.id){x+=n.x;y+=n.y;n=n.parent;}return {x,y};}
for(const name of ['macOS / main','macOS / edit']){
 const b=board(name),lines=b.findAll(n=>n.name==='Toolbar boundary').map(n=>pos(n,b).y);
 measurements[name+' / toolbar boundaries']=lines;
 if(lines.length!==3||new Set(lines).size!==1)errors.push(name+': toolbar boundaries differ');
 const p=b.children.find(n=>['Entry detail','Editor'].includes(n.name));
 if(p.findAll(n=>n.type==='TEXT'&&['Поле','Значение','Редактирование'].includes(n.characters)).length)errors.push(name+': obsolete heading');
 const labels=['Название','Логин','Пароль','URL','Теги','Заметки','Срок действия'];
 measurements[name+' / values']=labels.map(label=>{
  const row=p.children.find(n=>n.name==='Description row / '+label);
  const text=name.endsWith('/ edit')?p.findAll(n=>n.name==='Input / row / '+label+' / rest')[0].children.find(n=>n.type==='TEXT'):p.children.find(n=>n.type==='TEXT'&&n.x===184&&n.y>=row.y&&n.y<row.y+44);
  return {label,...pos(text,p)};
 });
 const table=b.children.find(n=>n.name==='Table / entries');
 const names=table.children.filter(n=>n.name.startsWith('Entry row / ')).map(n=>n.name.slice(12));
 if(names.join('|')!=='Cloudflare|Figma|GitHub|Linear|Notion|Vercel')errors.push(name+': default sort order');
 const selected=table.children.find(n=>n.name==='Entry row / GitHub');
 if(Math.abs(selected.fills[0].color.r-41/255)>0.001)errors.push(name+': selected entry lost');
}
if(JSON.stringify(measurements['macOS / main / values'])!==JSON.stringify(measurements['macOS / edit / values']))errors.push('view/edit value positions differ');
for(const b of boards.filter(n=>n.name.startsWith('macOS / '))){
 const t=b.children.find(n=>n.name==='Table / entries');if(!t)continue;
 if(t.children.filter(n=>n.name==='Column header separator').length!==2)errors.push(b.name+': missing header separators');
 if(t.children.filter(n=>n.name.startsWith('Sort / ')).length!==3)errors.push(b.name+': missing sort controls');
}
for(const n of figma.root.findAll(n=>n.name.startsWith('Input / '))){
 if(n.name.includes(' / row / ')&&n.fills.length)errors.push(n.name+': unexpected row fill');
 if(n.name.includes(' / standalone / ')&&!n.fills.length)errors.push(n.name+': missing standalone fill');
 if(n.name.endsWith(' / rest')&&n.strokes.length)errors.push(n.name+': resting border');
}
if(board('macOS / settings').findAll(n=>n.type==='TEXT'&&n.characters==='Сохранить').length)errors.push('settings: obsolete save action');
// Safety-relevant visual contracts for the v1 desktop scenarios.
const required = ['macOS / trash','macOS / pending','macOS / history-compare','macOS / receive',
 'macOS / settings-database','macOS / settings-locked','macOS / new-entry',
 'Dialog / import-review','Dialog / transfer-report','Dialog / restore-backup','Dialog / migrate',
 'Dialog / transfer-control','Dialog / accept-control','Dialog / recover-control',
 'States / session','States / exchange','States / conflicts','States / compatibility',
 'QA / minimum-1100-720','QA / light-main','QA / light-create','Flow / macOS-v1'];
for(const name of required)if(!board(name))errors.push('v1: missing '+name);
const hasText=(b,value)=>b.findAll(n=>n.type==='TEXT'&&n.characters.includes(value)).length>0;
for(const name of ['Dialog / revoke','Dialog / change-password']) {
 const b=board(name);
 if(!hasText(b,'Новый пароль')||!hasText(b,'Повтор пароля'))errors.push(name+': new password and confirmation required');
}
const resolver=board('macOS / conflict');
if(resolver.findAll(n=>n.name.startsWith('Radio / ')).some(n=>n.findAll(c=>c.type==='ELLIPSE'&&c.width===8).length))errors.push('conflicts: implicit initial choice');
if(!resolver.findAll(n=>n.name==='Button / disabled / Resolve conflict').length)errors.push('conflicts: initial commit must be disabled');
if(resolver.findAll(n=>n.name==='Icon button / Копировать вариант').length!==2)errors.push('conflicts: both original values need separate copy actions');
if(!board('Dialog / invite').findAll(n=>n.name==='QR / synthetic invitation').length)errors.push('invitation: QR missing');
const qr=board('Dialog / invite').findAll(n=>n.name==='QR / synthetic invitation')[0];
const modules=qr?.children.find(n=>n.type==='VECTOR');
if(!modules||modules.x<0||modules.y<0||modules.x+modules.width>qr.width||modules.y+modules.height>qr.height)errors.push('invitation: QR modules outside quiet-zone frame');
if(!hasText(board('Dialog / invite'),'TAYPEER-DEMO-'))errors.push('invitation: synthetic long code missing');
for(const name of ['macOS / unlock','macOS / receive','macOS / settings-locked']) {
 if(board(name).findAll(n=>['Sidebar / groups','Table / entries','Entry detail','Editor'].includes(n.name)).length)errors.push(name+': decrypted contents exposed');
}
if(!hasText(board('macOS / settings'),'Системная'))errors.push('settings: system theme default missing');
for(const name of ['macOS / welcome','macOS / settings-locked','macOS / receive']) {
 if(hasText(board(name),'Синхронизировано')||hasText(board(name),'2,4 МиБ'))errors.push(name+': no active database footer expected');
}
const minimum=board('QA / minimum-1100-720');
if(minimum.width!==1100||minimum.height!==720)errors.push('minimum window: wrong dimensions');
for(const p of figma.root.children) {
 const frames=p.children.filter(n=>n.type==='FRAME');
 for(let i=0;i<frames.length;i++)for(let j=i+1;j<frames.length;j++){
  const a=frames[i],b=frames[j];
  if(a.x<b.x+b.width&&b.x<a.x+a.width&&a.y<b.y+b.height&&b.y<a.y+a.height)errors.push('overlapping artboards: '+a.name+' / '+b.name);
 }
}
return {overflow,narrow_text:tiny,design_contracts:{errors,measurements}};
