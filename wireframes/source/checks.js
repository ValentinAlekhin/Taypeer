// Evaluate the saved FIG, after serialization has resolved geometry.
// Only a scroll track may exceed its explicitly clipped viewport. Its children
// still undergo the regular bounds check; arbitrary clipping is not an escape.
function scrollTrack(n) {
 return n.parent?.clipsContent && ((n.name==='Android / scroll content' && ['Android / scroll viewport','Android / drawer viewport'].includes(n.parent.name) && n.x===0 && n.width===n.parent.width && n.y<=0 && n.y+n.height>=n.parent.height) || (n.name==='Android / tab content' && n.parent.name==='Android / tabs viewport' && n.y===0 && n.height===n.parent.height) || (n.name==='Entry table / scroll content' && n.parent.name==='Entry table / scroll viewport' && n.x===0 && n.y===0 && n.width>=n.parent.width && n.height===n.parent.height) || (n.name==='Settings / scroll content' && n.parent.name==='Settings / scroll viewport' && n.x===0 && n.y===0 && n.width===n.parent.width && n.height>=n.parent.height));
}
const overflow=figma.root.findAll(n=>n.parent && !scrollTrack(n) && !['PAGE','CANVAS','DOCUMENT'].includes(n.parent.type) && !['VECTOR','INSTANCE'].includes(n.type) && !['INSTANCE','COMPONENT'].includes(n.parent.type) && (n.x < -1 || n.y < -1 || n.x+n.width>n.parent.width+1 || n.y+n.height>n.parent.height+1)).map(n=>({name:n.name,parent:n.parent.name,x:n.x,y:n.y,w:n.width,h:n.height,pw:n.parent.width,ph:n.parent.height}));
const tiny=figma.root.findAll(n=>n.type==='TEXT'&&n.characters.length>4&&n.width<28).map(n=>({name:n.name,width:n.width}));
const errors=[], measurements={};
if(figma.root.findAll(n=>/pass2p/i.test(n.name)||(n.type==='TEXT'&&/pass2p/i.test(n.characters))).length)errors.push('branding: obsolete product name');
if(!figma.root.findAll(n=>n.type==='TEXT'&&n.characters==='Taypeer').length)errors.push('branding: product name missing');
const documentPages=figma.root.children.filter(p=>p.name!=='Internal Only Canvas'||p.children.length);
const expectedPages=['00 · Начать здесь','01 · Desktop','02 · Android','03 · Компоненты'];
if(JSON.stringify(documentPages.map(p=>p.name))!==JSON.stringify(expectedPages))errors.push('document: expected four ordered platform pages');
for(const page of documentPages)if(page.children.some(n=>!['FRAME','TEXT'].includes(n.type)))errors.push(page.name+': artboards must remain flat frames');
const boards=documentPages.flatMap(p=>p.children.filter(n=>n.type==='FRAME'));
if(boards.length!==172)errors.push('document: expected 172 artboards');
if(new Set(boards.map(n=>n.name)).size!==boards.length)errors.push('document: duplicate artboard names');
for(const b of boards) {
 const expected=b.name==='Taypeer / обзор'?expectedPages[0]:
  ['System / components','Android states / components'].includes(b.name)?expectedPages[3]:
  b.name.startsWith('Android')?expectedPages[2]:
  /^(macOS|Dialog|Tabs|Menus|States|QA|Flow) \/ /.test(b.name)?expectedPages[1]:null;
 if(!expected||b.parent.name!==expected)errors.push('document: misplaced artboard '+b.name);
}
const componentIds=new Set(figma.root.findAll(n=>n.type==='COMPONENT').map(n=>n.id));
for(const n of figma.root.findAll(n=>n.type==='INSTANCE'))if(!n.mainComponent||!componentIds.has(n.mainComponent.id))errors.push('components: broken instance '+n.name);
for(const n of figma.root.findAll(n=>n.type==='COMPONENT')) {
 let owner=n.parent;while(owner&&!['PAGE','CANVAS'].includes(owner.type))owner=owner.parent;
 if(owner?.name!==expectedPages[3])errors.push('components: master outside library '+n.name);
}
measurements['Document / artboards per page']=Object.fromEntries(documentPages.map(p=>[p.name,p.children.filter(n=>n.type==='FRAME').length]));
const navigationCounts={'01 · Desktop':11,'02 · Android':14,'03 · Компоненты':2};
for(const p of documentPages.filter(p=>navigationCounts[p.name])) {
 const children=p.children;
 const headings=children.filter(n=>n.type==='TEXT'&&n.name.startsWith('Раздел / '));
 if(headings.length!==navigationCounts[p.name]||children[0]?.id!==headings[0]?.id)errors.push(p.name+': navigation headings missing');
 headings.forEach((h,i)=>{
  const next=headings[i+1],start=children.findIndex(n=>n.id===h.id)+1,end=next?children.findIndex(n=>n.id===next.id):children.length;
  const members=children.slice(start,end).filter(n=>n.type==='FRAME');
  const title=h.name.slice('Раздел / '.length);
  if(!title.startsWith(String(i+1).padStart(2,'0')+' · ')||h.characters!==title+' ('+members.length+')'||!h.visible||!members.length)errors.push(p.name+': invalid navigation heading '+h.name);
  if(members.some(n=>n.y<h.y+h.height+24||(next&&n.y+n.height>next.y-80)))errors.push(p.name+': navigation group overlaps heading '+h.name);
 });
 measurements[p.name+' / navigation groups']=headings.map(h=>h.characters);
}
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
 const names=table.findAll(n=>n.name.startsWith('Entry row / ')).map(n=>n.name.slice(12));
 if(names.join('|')!=='Cloudflare|Figma|GitHub|Linear|Notion|Vercel')errors.push(name+': default sort order');
 const selected=table.findAll(n=>n.name==='Entry row / GitHub')[0];
 const overlay=selected.children.find(n=>n.name==='Table selection / translucent');
 if(!overlay||Math.abs(overlay.fills[0].opacity-0.2)>0.001)errors.push(name+': selected entry lost');
 if(b.children.find(n=>n.name==='TitleBar').height!==34||p.width!==520)errors.push(name+': stale window chrome or initial card width');
 if(!table.findAll(n=>n.name==='Table header'&&n.height===32).length||table.findAll(n=>n.name.startsWith('Entry row / ')&&n.height!==32).length)errors.push(name+': table differs from product DataTable density');
}
if(JSON.stringify(measurements['macOS / main / values'])!==JSON.stringify(measurements['macOS / edit / values']))errors.push('view/edit value positions differ');
for(const b of boards.filter(n=>n.name.startsWith('macOS / '))){
 const t=b.children.find(n=>n.name==='Table / entries');if(!t)continue;
 if(t.findAll(n=>n.name==='Column header separator').length!==3)errors.push(b.name+': missing header separators');
 if(t.findAll(n=>n.name.startsWith('Sort / ')).length!==3)errors.push(b.name+': missing sort controls');
}
for(const n of figma.root.findAll(n=>n.name.startsWith('Input / '))){
 if(n.name.includes(' / row / ')&&n.fills.length)errors.push(n.name+': unexpected row fill');
 if(n.name.includes(' / standalone / ')&&!n.fills.length)errors.push(n.name+': missing standalone fill');
 if(n.name.endsWith(' / rest')&&n.strokes.length)errors.push(n.name+': resting border');
}
if(board('macOS / settings').findAll(n=>n.type==='TEXT'&&n.characters==='Сохранить').length)errors.push('settings: obsolete save action');
for(const name of ['macOS / settings','macOS / settings-database','macOS / settings-locked']) {
 const b=board(name), pageName=name.endsWith('settings-database')?'База данных':'Устройство';
 if(b.findAll(n=>n.name==='Sidebar / groups'||n.name==='Table / entries').length)errors.push(name+': settings must use the full working area');
 for(const required of ['Sidebar / settings','Settings search','Settings / scroll viewport','Settings / scroll content','Setting page / '+pageName])if(b.findAll(n=>n.name===required).length!==1)errors.push(name+': missing native settings structure '+required);
 if(!b.findAll(n=>n.name.startsWith('Setting group / ')).length)errors.push(name+': setting groups missing');
}
if(board('macOS / main').findAll(n=>n.name.startsWith('Icon button / Копировать ')&&n.visible).length)errors.push('entry view: copy actions must remain hidden at rest');
// Safety-relevant visual contracts for the v1 desktop scenarios.
const required = ['macOS / trash','macOS / pending','macOS / history-compare','macOS / receive',
 'macOS / settings-database','macOS / settings-locked','macOS / new-entry',
 'macOS / drafts','macOS / alternatives',
 'Dialog / transfer-control','Dialog / accept-control','Dialog / recover-control',
 'States / session','States / exchange','States / alternatives','States / compatibility',
 'QA / minimum-1100-720','QA / light-main','QA / light-create','Flow / macOS-v1'];
for(const name of required)if(!board(name))errors.push('v1: missing '+name);
const hasText=(b,value)=>b.findAll(n=>n.type==='TEXT'&&n.characters.includes(value)).length>0;
for(const name of ['Dialog / revoke','Dialog / change-password']) {
 const b=board(name);
 if(!hasText(b,'Новый пароль')||!hasText(b,'Повтор пароля'))errors.push(name+': new password and confirmation required');
}
const alternatives=board('macOS / alternatives');
if(alternatives.findAll(n=>n.name==='Icon button / Копировать вариант').length!==2)errors.push('history alternatives: independent copy missing');
if(alternatives.findAll(n=>n.name.startsWith('Radio / ')).length)errors.push('history alternatives: mandatory selection remains');
const duplicateAttributes=board('States / alternatives').findAll(n=>n.name==='Component / Совпадающие ключи / защита атрибута')[0];
if(!duplicateAttributes||!hasText(duplicateAttributes,'ID a1')||!hasText(duplicateAttributes,'ID b2')||duplicateAttributes.findAll(n=>n.name==='Icon button / Копировать вариант').length!==2||hasText(duplicateAttributes,'Применить'))errors.push('attributes: duplicate names must remain separate without forced resolution');
if(figma.root.findAll(n=>n.type==='TEXT'&&['Разрешить конфликт','Выбрать победителя','Обновить формат'].includes(n.characters)).length)errors.push('automation: obsolete manual conflict or migration action');
if(board('macOS / empty').findAll(n=>n.name==='Icon button / Создать запись').length||!board('macOS / empty').findAll(n=>n.name==='Icon button / Создать запись / недоступно'&&n.opacity<0.5).length)errors.push('groups: creation must be unavailable without an active group');
const picker=board('Dialog / color-picker');
for(const name of ['ColorPicker / popover / Palette','ColorPicker / mode tabs','ColorPicker / palette grid','ColorPicker / HEX input'])if(!picker?.findAll(n=>n.name===name).length)errors.push('color picker: native part missing '+name);
if(picker?.findAll(n=>n.type==='TEXT'&&['Выбрать','Сохранить','Отмена'].includes(n.characters)).length)errors.push('color picker: selection belongs to the entry draft, not a separate modal save');
if(!board('System / components').findAll(n=>n.name==='ColorPicker / Alpha slider').length)errors.push('color picker: HSLA alpha specimen missing');
const forbiddenBoards=['Dialog / restore-backup','Dialog / import-review','Dialog / export-kdbx','Dialog / transfer-report','Dialog / migrate','Dialog / delete-objects','macOS / backups','macOS / transfer','macOS / conflict','Android / draft','Android / delete','Android / files','Android / conflict','Android / conflicts','Android form / import','Android form / export','Android form / migrate'];
for(const name of forbiddenBoards)if(board(name))errors.push('automation: obsolete route '+name);
if(figma.root.findAll(n=>n.type==='TEXT'&&/KDBX|Без группы|Изменения сохраняются автоматически|Резервные копии/.test(n.characters)).length)errors.push('automation: obsolete product command');
if(!hasText(board('macOS / drafts'),'Новая запись')||!board('Dialog / unsaved'))errors.push('manual saving: draft list or dirty decision missing');
if(!board('macOS / edit').findAll(n=>n.name==='Icon button / Сохранить запись').length||!board('macOS / edit').findAll(n=>n.name==='Icon button / Отменить правки').length)errors.push('manual saving: entry save/cancel missing');
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
 if(hasText(board(name),'Синхронизировано')||board(name).findAll(n=>n.type==='TEXT'&&/^2,4 М(?:и)?Б$/.test(n.characters)).length)errors.push(name+': no active database footer expected');
}
const minimum=board('QA / minimum-1100-720');
if(minimum.width!==1100||minimum.height!==720)errors.push('minimum window: wrong dimensions');
// Android phone contracts: intrinsic touch sizes and actual visible rectangles.
const android=boards.filter(n=>n.name.startsWith('Android'));
const androidRequired=['entries','drawer','drawer-scrolled','empty','search-current','search-all','search-empty','entry','edit','new-entry','advanced','advanced-edit','appearance','appearance-edit','properties','history','history-version','history-compare','trash','devices','receive','receiving','alternatives','drafts','pending','pending-source','settings','settings-database','welcome','unlock'];
for(const name of androidRequired)if(!board('Android / '+name))errors.push('android: missing '+name);
function ancestor(n,root){while(n){if(n.id===root.id)return true;n=n.parent;}return false;}
function visibleRect(n,b) {
 const p=pos(n,b);let r={x:p.x,y:p.y,right:p.x+n.width,bottom:p.y+n.height};
 for(let a=n.parent;a&&a.id!==b.id;a=a.parent)if(a.clipsContent){const p=pos(a,b);r={x:Math.max(r.x,p.x),y:Math.max(r.y,p.y),right:Math.min(r.right,p.x+a.width),bottom:Math.min(r.bottom,p.y+a.height)};}
 return r.right>r.x&&r.bottom>r.y?r:null;
}
let touchCount=0;
for(const b of android) {
 const overlay=b.children?.find(n=>['Android / drawer viewport','Android / bottom sheet'].includes(n.name));
 const touches=b.findAll(n=>n.name.startsWith('Touch / '));touchCount+=touches.length;
 for(const n of touches)if(n.width<48||n.height<48)errors.push(b.name+': touch below 48 dp: '+n.name);
 const visible=touches.filter(n=>!overlay||ancestor(n,overlay)).map(n=>({n,r:visibleRect(n,b)})).filter(v=>v.r);
 for(let i=0;i<visible.length;i++)for(let j=i+1;j<visible.length;j++){
  const {n:a,r:x}=visible[i],{n:b2,r:y}=visible[j];
  if(x.x<y.right-0.1&&y.x<x.right-0.1&&x.y<y.bottom-0.1&&y.y<x.bottom-0.1)errors.push(b.name+': overlapping touch actions: '+a.name+' / '+b2.name);
 }
 if(b.width<=844&&b.height<=932) {
  for(const {n,r} of visible)if(r.y<32||r.bottom>b.height-24)errors.push(b.name+': action in system inset: '+n.name);
  const form=b.children.find(n=>n.name==='Android / form actions'), viewport=b.children.find(n=>n.name==='Android / scroll viewport'),ime=b.children.find(n=>n.name==='System / IME illustration');
  if(form&&viewport.y+viewport.height>form.y)errors.push(b.name+': form content behind confirmation');
  if(form&&ime&&form.y+form.height!==ime.y)errors.push(b.name+': confirmation not above IME');
 }
 if(b.findAll(n=>n.name.startsWith('Touch / ')&&(/Автоматическая.*копия|Восстановить снимок|Резервные копии/.test(n.name))).length)errors.push(b.name+': deferred backup flow');
}
measurements['Android / touch targets']=touchCount;
for(const name of ['Android / edit','Android / entry']) {
 const b=board(name);measurements[name+' / values']=b.findAll(n=>n.name.startsWith('Value / ')).map(n=>({name:n.name,...pos(n,b)}));
 if(b.findAll(n=>n.name.startsWith('Touch / ')&&n.name.includes('Вкладка ')).length!==5)errors.push(name+': five scrollable tabs required');
}
if(JSON.stringify(measurements['Android / entry / values'])!==JSON.stringify(measurements['Android / edit / values']))errors.push('android: view/edit values moved');
for(const name of ['Android form / create','Android form / change-password','Android form / revoke','Android form / recover-control']) {
 const b=board(name);if(!b||!hasText(b,'Повтор пароля')||!hasText(b,name.endsWith('/ create')?'Мастер-пароль':'Новый пароль'))errors.push(name+': password confirmation missing');
}
const mobileAlternatives=board('Android / alternatives');
if(mobileAlternatives.findAll(n=>n.name==='Touch / ghost / Копировать вариант').length!==2)errors.push('android alternatives: independent copy missing');
if(!board('Android / edit').findAll(n=>n.name==='Android / form actions').length)errors.push('android manual saving: editor footer missing');
if(!hasText(board('Android / edit'),'Сохранить')||!hasText(board('Android / edit'),'Отмена')||!hasText(board('Android / drafts'),'Новая запись')||!board('Android form / unsaved'))errors.push('android manual saving: actions, drafts or dirty decision missing');
for(const name of ['Android / unlock','Android / receive','Android / receiving']) {
 const b=board(name);if(b.findAll(n=>n.name==='Value / Логин'||n.name==='Value / Пароль'||n.name==='Android / group row').length)errors.push(name+': decrypted contents on locked route');
}
for(const [name,w,h] of [['small-360-640',360,640],['large-430-932',430,932],['landscape-844-390',844,390]]){const b=board('Android QA / '+name);if(!b||b.width!==w||b.height!==h)errors.push('android: control dimensions '+name);}
const mobileQr=board('Android form / invite')?.findAll(n=>n.name==='QR / synthetic invitation')[0];
if(!mobileQr||!mobileQr.findAll(n=>n.type==='VECTOR').length)errors.push('android invitation: editable QR missing');
if(!hasText(board('Android states / exchange'),'Изменения получены')||!hasText(board('Android states / exchange'),'Изменения применены'))errors.push('android: receipt/application distinction missing');
if(!hasText(board('Android form / create'),'Описание'))errors.push('android create: optional description missing');
if(board('Android / pending-source').findAll(n=>n.name.startsWith('Touch / ')&&n.name.includes('Удалить источник')).length)errors.push('android pending: authenticated late edits cannot be deleted as an unaccepted packet');
if(!hasText(board('Android / database-switcher'),'Сохранить копию'))errors.push('android files: explicit copy route missing');
for(const name of ['text-200-form','text-200-keyboard']) {
 const actions=board('Android QA / '+name).findAll(n=>n.name==='Android / form actions')[0];
 if(!actions||actions.findAll(n=>n.type==='TEXT'&&n.height<Math.ceil(n.fontSize*1.4)).length)errors.push('android large text: confirmation label clips '+name);
}
for(const p of figma.root.children) {
 const frames=p.children.filter(n=>n.type==='FRAME');
 for(let i=0;i<frames.length;i++)for(let j=i+1;j<frames.length;j++){
  const a=frames[i],b=frames[j];
  if(a.x<b.x+b.width&&b.x<a.x+a.width&&a.y<b.y+b.height&&b.y<a.y+a.height)errors.push('overlapping artboards: '+a.name+' / '+b.name);
 }
}
return {overflow,narrow_text:tiny,design_contracts:{errors,measurements}};
