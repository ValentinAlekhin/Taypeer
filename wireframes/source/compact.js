// Build through scripts/design.py using assets/lucide-base.fig.
// Explicit geometry keeps the editable FIG stable across OpenPencil round trips.
// Both palettes are injected from resources/theme.toml by scripts/design.py.
const P=DESIGN_PALETTES.dark;
const LIGHT_PALETTE=DESIGN_PALETTES.light;
const rgb=h=>({r:parseInt(h.slice(1,3),16)/255,g:parseInt(h.slice(3,5),16)/255,b:parseInt(h.slice(5,7),16)/255});
const paint=c=>[{type:'SOLID',color:rgb(P[c]||c)}];
const sourceIcons=figma.root.findAll(n=>n.type==='COMPONENT'&&n.name.startsWith('Icon / lucide:'));
// EXTRA_ICONS is injected from assets/lucide-extra.json by scripts/design.py.
for(const def of EXTRA_ICONS){const n=figma.createComponent();n.name=def.name;n.resize(24,24);n.fills=[];const v=figma.createVector();n.appendChild(v);v.vectorPaths=def.paths;v.fills=[];v.strokes=paint('muted');v.strokeWeight=2;v.strokeCap='ROUND';v.strokeJoin='ROUND';sourceIcons.push(n);}
const oldPages=[...figma.root.children];
const pages={};for(const name of ['00 · Начать здесь','01 · macOS','02 · Android','03 · Диалоги','04 · Компоненты','05 · Вкладки записи']){const p=figma.createPage();p.name=name;pages[name]=p;}
const library=pages['04 · Компоненты'];
const icons={};
for(const n of sourceIcons){library.appendChild(n);icons[n.name.split(':')[1]]=n;n.layoutMode='NONE';n.resize(16,16);for(const v of n.findAll(x=>x.type==='VECTOR')){v.x*=2/3;v.y*=2/3;v.vectorPaths=v.vectorPaths.map(p=>({...p,data:p.data.replace(/-?\d*\.?\d+(?:e[-+]?\d+)?/gi,s=>String(Number(s)*2/3))}));v.strokes=paint('muted');v.strokeWeight=1.4;v.strokeCap='ROUND';v.strokeJoin='ROUND';}}
for(const p of oldPages)p.remove();
for(const c of figma.getLocalVariableCollections())figma.deleteVariableCollection(c.id);
const collection=figma.createVariableCollection('Taypeer · semantic / dark');
const vars={};for(const [k,h] of Object.entries(P))vars[k]=figma.createVariable(k,'COLOR',collection.id,{...rgb(h),a:1});
function fill(n,c){n.fills=c?paint(c):[];if(vars[c])figma.bindVariable(n.id,'fills/0/color',vars[c].id);}
function box(p,name,x,y,w,h,c='bg',r=0,border=false){const n=figma.createFrame();p.appendChild(n);n.name=name;n.layoutMode='NONE';n.x=x;n.y=y;n.resize(w,h);fill(n,c);n.cornerRadius=r;n.clipsContent=false;if(border){n.strokes=paint('border');n.strokeWeight=1;}return n;}
function text(p,s,x,y,w,size=13,c='fg',weight=400,h=20,mono=false){const n=figma.createText();p.appendChild(n);n.name=s.slice(0,64);n.characters=s;n.fontName={family:'Inter',style:weight>=600?'Semi Bold':weight>=500?'Medium':'Regular'};n.fontSize=size;n.textAutoResize='NONE';n.resize(w,Math.max(h,Math.ceil(size*1.4)));n.x=x;n.y=y;fill(n,c);return n;}
function rule(p,x,y,w,h=1){return box(p,'Separator',x,y,w,h,'border');}
function icon(p,key,x,y,c='muted'){key=key==='settings'?'sliders-horizontal':key;const n=icons[key].createInstance();p.appendChild(n);n.x=x;n.y=y;n.name='Icon / '+key;if(c!=='muted')for(const v of n.findAll(v=>v.type==='VECTOR')){v.strokes=paint(c);v.strokeWeight=1.4;v.strokeCap='ROUND';v.strokeJoin='ROUND';}return n;}
function button(p,s,x,y,w=100,kind='default',mobile=false,ico){const h=mobile?44:32;const n=box(p,'Button / '+kind+' / '+s,x,y,w,h,kind==='primary'?'primary':kind==='ghost'?'':kind==='disabled'?'panel':'chrome',4,kind==='default');if(ico)icon(n,ico,10,(h-16)/2);text(n,s,ico?34:10,(h-18)/2,w-(ico?40:20),13,kind==='primary'?'onPrimary':kind==='danger'?'danger':kind==='disabled'?'dim':'fg',500,18);return n;}
function ib(p,key,x,y,m=false){const n=box(p,'Icon button / '+key,x,y,m?44:32,m?44:32,'',4);icon(n,key,m?14:8,m?14:8);return n;}
const UI={title:34,toolbar:44,tab:38,row:44,label:144,gap:16,inset:24,value:184,inspector:520};
function inputFrame(p,name,x,y,w,h=32,variant='standalone',state='rest'){
 const n=box(p,'Input / '+variant+' / '+name+' / '+state,x,y,w,h,variant==='row'?'':'chrome',4);
 if(state==='focus'||state==='error'){n.strokes=paint(state==='focus'?'fg':'danger');n.strokeWeight=state==='focus'?2:1;}
 return n;
}
function field(p,label,value,x,y,w=360,{error=false,secret=false,m=false,select=false,variant='standalone',state='rest'}={}){
 const ix=m?x:x+UI.label+UI.gap,iy=m?y+24:y,iw=m?w:w-UI.label-UI.gap,pad=variant==='row'?0:10;
 text(p,label,x,m?y:y+6,m?w:UI.label,m?12:13,error?'danger':'muted');
 const f=inputFrame(p,label,ix,iy,iw,m?44:32,variant,error?'error':state);
 if(select)f.name='Select / '+label;
 text(f,value,pad,m?12:6,iw-pad-(secret||select?44:10),13,'fg',400,20,secret);
 if(secret)ib(f,'eye',iw-(m?44:36),0,m).name='Icon button / Показать пароль';
 if(select)icon(f,'chevron-down',iw-28,m?14:8);
 return f;
}
function entryHeader(p,w,edit=false,m=false){
 const h=m?56:UI.toolbar,sz=m?44:32,pad=m?16:UI.inset,yy=(h-sz)/2;
 const title=text(p,'GitHub',pad,(h-20)/2,w-(m?pad+112:edit?232:128),18,'fg',500);title.name='Entry title';
 if(edit){
  text(p,'Сохранено',m?w-pad-120:w-200,yy+6,m?120:96,12,'muted');
  if(!m)ib(p,'arrow-left',w-96,yy).name='Icon button / Назад из редактора';
 }else ib(p,'pencil',m?w-pad-sz:w-96,yy,m).name='Icon button / Редактировать запись';
 if(!m)ib(p,'ellipsis',w-56,yy).name='Icon button / Действия записи';
 rule(p,0,h,w).name='Toolbar boundary';
 return h;
}
function toggle(p,s,x,y,w,on=true,m=false){text(p,s,x,y,w-52,13);const b=box(p,'Switch / '+s,x+w-36,y,32,18,on?'primary':'border',9);box(b,'Thumb',on?17:3,3,12,12,on?'onPrimary':'muted',6);}
function radio(p,s,x,y,w,selected=false){const n=box(p,'Radio / '+s,x,y,w,36,'',0);const e=figma.createEllipse();n.appendChild(e);e.x=0;e.y=8;e.resize(16,16);fill(e,'bg');e.strokes=paint(selected?'fg':'muted');e.strokeWeight=1;if(selected){const a=figma.createEllipse();n.appendChild(a);a.x=4;a.y=12;a.resize(8,8);fill(a,'fg');}text(n,s,28,7,w-28);}
const counts={};const boards=[];
function board(page,name,w,h){const p=pages[page];figma.currentPage=p;const i=counts[page]||0;counts[page]=i+1;const cols=page.includes('Android')?4:page.includes('Диалоги')?3:2;const cw=page.includes('Android')?470:w+80,ch=page.includes('Диалоги')?570:h+80;const n=box(p,name,(i%cols)*cw,Math.floor(i/cols)*ch,w,h,'bg',8,true);n.clipsContent=true;boards.push({name,id:n.id,page});return n;}
function shell(name,{m=false,w=1320,h=820,empty=false,destination='01 · macOS'}={}){
const p=board(m?'02 · Android':destination,name,m?w===1320?390:w:w,m?844:h);const W=p.width,H=p.height,isWelcome=["/ welcome","/ receive","/ settings-locked"].some(s=>name.endsWith(s));
box(p,'TitleBar',0,0,W,m?88:UI.title,'chrome');
if(m){text(p,'9:41',16,8,100,11);text(p,'Wi-Fi · 100%',W-108,8,100,11,'muted');icon(p,'file-key-2',16,51);text(p,'Рабочая',44,49,W-144,14,'fg',500);ib(p,'chevron-down',W-94,36,true);ib(p,'settings',W-48,36,true);}
else{['#EA6B66','#D6B86C','#80B981'].forEach((c,i)=>{const e=figma.createEllipse();p.appendChild(e);e.x=16+i*19;e.y=12;e.resize(11,11);fill(e,c);});icon(p,'file-key-2',W/2-74,9);text(p,isWelcome?'Taypeer':'Рабочая.taypeer',W/2-50,7,156,13,'fg',500);if(!isWelcome)ib(p,'chevron-down',W/2+110,1);ib(p,'settings',W-40,1);}
rule(p,0,m?88:UI.title,W);
if(!m){box(p,'StatusBar',0,H-28,W,28,'panel');rule(p,0,H-28,W);if(!isWelcome){icon(p,'database',12,H-22);text(p,'2,4 МиБ',36,H-22,140,11,'muted');text(p,'Синхронизировано',W-206,H-22,166,11,'muted').textAlignHorizontal='RIGHT';icon(p,'refresh-cw',W-28,H-22);}}
else{box(p,'Home indicator',W/2-48,H-10,96,3,'muted',2);}return p;}
function sidebar(p,empty=false,active='Работа'){const n=box(p,'Sidebar / groups',0,UI.title+1,224,p.height-UI.title-29,'panel');ib(n,'folder-plus',180,6);ib(n,'ellipsis',140,6);rule(n,0,UI.toolbar,224).name='Toolbar boundary';if(empty){text(n,'Нет групп',16,67,180,13,'muted');button(n,'Добавить группу…',16,99,190);}else{[['Работа',0,'6'],['Сервисы',1,'2'],['Инфраструктура',1,'2'],['Личное',0,'3'],['Архив',0,'0']].forEach(([s,level,count],i)=>{const y=52+i*34;box(n,'Tree row / '+s,8,y,208,32,s===active?'selected':'',4);icon(n,level?'chevron-right':'chevron-down',16+level*16,y+8);icon(n,'folder',40+level*16,y+8);text(n,s,64+level*16,y+7,level?117:138,12,'fg',s===active?500:400);text(n,count,194,y+7,20,11,'muted');});}rule(n,0,n.height-172,224);[['file','Без группы'],['trash-2','Корзина'],['monitor','Устройства'],['history','Черновики']].forEach(([key,s],i)=>{icon(n,key,16,n.height-153+i*36);text(n,s,44,n.height-155+i*36,164,13,'muted');});rule(p,224,UI.title+1,1,n.height);}
const entries=[['Cloudflare','admin@studio.dev'],['Figma','valentin@studio.dev'],['GitHub','valentin'],['Linear','valentin'],['Notion','valentin@studio.dev'],['Vercel','valentin']];
// Keep the editable mockup aligned with the product Entries/DataTable builder.
// Fixed column widths overflow a clipped viewport rather than shrinking or hiding columns.
function collectionPane(p,search=false,selected=true,empty=false,{font=16,side=224,detail=UI.inspector}={}){
 const x=side+1,w=selected?p.width-x-detail-1:p.width-x,h=p.height-UI.title-29,n=box(p,'Table / entries',x,UI.title+1,w,h,'panel');
 const scopeWidth=136,fontScale=font/16;
 const searchWidth=w-16-12-64-scopeWidth;
 const f=inputFrame(n,'Search',8,6,searchWidth);icon(f,'search',10,8);text(f,search?'git':'Поиск',36,6,searchWidth-64,14,search?'fg':'muted');if(search)ib(f,'x',searchWidth-32,0).name='Icon button / Очистить поиск';
 const scope=button(n,search?'Все открытые':'Эта база',searchWidth+12,6,scopeWidth,'ghost');icon(scope,'chevron-down',scopeWidth-24,8);scope.name='Search scope / '+(search?'all-unlocked':'current');scope.children.find(n=>n.type==='TEXT').resize(scopeWidth-40,18);
 ib(n,'file-plus-2',w-76,6).name='Icon button / Создать запись';ib(n,'ellipsis',w-40,6).name='Icon button / Колонки';rule(n,0,UI.toolbar,w).name='Toolbar boundary';
 const viewport=box(n,'Entry table / scroll viewport',0,UI.toolbar,w,h-UI.toolbar-32,'panel');viewport.clipsContent=true;
 const columns=[['Название',200*fontScale],['Логин',180*fontScale],['База / группа',240*fontScale]];
 const contentWidth=Math.max(w,columns.reduce((sum,c)=>sum+c[1],0)+12*fontScale);
 const content=box(viewport,'Entry table / scroll content',0,0,contentWidth,viewport.height,'panel');
 box(content,'Table header',0,0,contentWidth,32,'chrome');
 let xx=0;columns.forEach(([label,width],i)=>{
  text(content,label,xx+12*fontScale,6,width-48*fontScale,font,'muted');
  const sort=ib(content,i===0?'arrow-up':'chevrons-up-down',xx+width-32,0);sort.name='Sort / '+label+' / '+(i===0?'ascending':'inactive');
  rule(content,xx+width-1,0,1,32).name='Column header separator';xx+=width;
 });rule(content,0,31,contentWidth);
 const rows=empty?[]:search?[['GitHub','valentin','Рабочая / Работа'],['GitLab','personal@mail.dev','Личная / Сервисы']]:entries.map(([title,user])=>[title,user,'Рабочая / Работа']);
 rows.forEach((values,i)=>{
  const y=32+i*32,r=box(content,'Entry row / '+values[0],0,y,contentWidth,32,'panel');
  let x=0;values.forEach((value,j)=>{const width=columns[j][1];if(j===0)icon(r,'key-round',x+12*fontScale,8);text(r,value,x+(j===0?36:12)*fontScale,6,width-(j===0?48:24)*fontScale,j===0?font:font*0.75,j===0?'fg':'muted');rule(r,x+width-1,0,1,32);x+=width;});
  rule(r,0,31,contentWidth);
  if(selected&&values[0]==='GitHub'){const overlay=box(r,'Table selection / translucent',0,0,contentWidth,32,'selected');overlay.fills=overlay.fills.map(paint=>({...paint,opacity:0.2}));overlay.strokes=paint('selected');overlay.strokeWeight=1;}
 });
 if(empty)text(content,'Записи не найдены',12,Math.round(content.height/2),contentWidth-24,font,'muted').textAlignHorizontal='CENTER';
 text(n,'Записи: '+rows.length,12,h-24,w-24,12,'muted');
 if(contentWidth>w){rule(viewport,0,viewport.height-4,w,3);box(viewport,'Horizontal scrollbar thumb',0,viewport.height-4,Math.max(40,w*w/contentWidth),3,'muted',2);}
 if(selected)rule(p,x+w,UI.title+1,1,h);
 return n;
}
function tabs(p,x,y,w,active='Обзор',m=false){const labels=['Обзор','Дополнительно','Вид','Свойства','История'];const widths=m?[66,120,50,92,78]:[70,134,60,100,90];let dx=0;rule(p,x,y+UI.tab-1,w);for(let i=0;i<labels.length;i++){if(dx>=w)break;const s=labels[i];text(p,s,x+dx+8,y+10,Math.min(widths[i]-10,w-dx-8),m?12:13,s===active?'fg':'muted',s===active?500:400);if(s===active)box(p,'Selected tab / '+s,x+dx,y+UI.tab-2,Math.min(widths[i],w-dx),2,'muted');dx+=widths[i];}}
function detail(p,{m=false,edit=false,advanced=false,width=UI.inspector}={}){
 const x=m?0:p.width-width,y=m?89:UI.title+1,w=m?p.width:width,n=box(p,edit?'Editor':'Entry detail',x,y,w,m?737:p.height-UI.title-29),inset=m?16:UI.inset;
 const h=entryHeader(n,w,edit,m);tabs(n,0,h+1,w,advanced?'Дополнительно':'Обзор',m);const top=m?h+UI.tab+17:h+UI.tab+1;
 if(advanced){
  text(n,'Атрибуты',inset,top,w-60,13,'fg',500);
  [['Recovery code','••••••••••••'],['Организация','Studio']].forEach(([a,b],i)=>{text(n,a,inset,top+42+i*72,w-80,12,'muted');text(n,b,inset,top+66+i*72,w-80,13);ib(n,'copy',w-60,top+54+i*72,m);rule(n,inset,top+102+i*72,w-inset*2);});
  text(n,'Вложения',inset,top+204,w-60,13,'fg',500);icon(n,'file',inset,top+249);text(n,'recovery-codes.txt',inset+28,top+247,w-100,13);text(n,'20 КиБ',inset+28,top+273,180,12,'muted');ib(n,'download',w-60,top+242,m);text(n,'2,4 / 100 МиБ',inset,top+326,w-50,11,'muted');return n;
 }
 const values=[['Название','GitHub'],['Логин','valentin'],['Пароль','••••••••••••••••'],['URL','https://github.com'],['Теги','работа, разработка'],['Срок действия','Бессрочно'],['Заметки','Основной рабочий аккаунт']];
 values.forEach(([label,val],i)=>{
  const yy=top+i*(m?72:UI.row),special=label==='Пароль'||label==='URL',sz=m?44:32,right=w-inset-sz;
  if(!m)box(n,'Description row / '+label,0,yy,w,UI.row,i%2?'panel':'bg');
  text(n,label,inset,yy+(m?0:12),m?w-100:UI.label,m?12:13,'muted');
  if(edit){
   const xx=m?inset:UI.value,iy=yy+(m?12:6),iw=w-inset-xx-(special?sz+8:0),input=inputFrame(n,label,xx,iy,iw,m?44:32,'row');
   text(input,val,0,m?12:6,iw-(label==='Пароль'?sz+8:8),13);
   if(label==='Пароль')ib(input,'eye',iw-sz,0,m).name='Icon button / Показать пароль';
   if(label==='Срок действия')icon(input,'chevron-down',iw-20,m?14:8);
   if(special)ib(n,label==='Пароль'?'dice-5':'download',right,yy+(m?12:6),m).name=label==='Пароль'?'Icon button / Генератор пароля…':'Icon button / Загрузить favicon по URL';
  }else{
   text(n,val,m?inset:UI.value,yy+(m?24:12),m?w-132:w-UI.value-inset-80,13);
   if(i>0&&i<4){const a=ib(n,'copy',right,yy+(m?12:6),m);a.name='Icon button / Копировать '+label;if(!m)a.visible=false;}
   if(label==='Пароль'){const a=ib(n,'eye',right-sz-8,yy+(m?12:6),m);a.name='Icon button / Показать пароль';if(!m)a.visible=false;}
  }
  rule(n,m?inset:0,yy+(m?63:UI.row-1),m?w-inset*2:w);
 });return n;
}
function workspace(kind='main',w=1320){const p=shell('macOS / '+kind,{w});sidebar(p,kind==='empty');const selected=!['empty','search','no-selection'].includes(kind);collectionPane(p,kind==='search',selected,kind==='empty');if(selected)detail(p,{edit:kind==='edit',advanced:kind==='advanced'});return p;}
workspace();workspace('empty');workspace('edit');workspace('no-selection');workspace('search');
function utility(name,title,{full=false}={}){const p=shell('macOS / '+name);if(!full)sidebar(p,false,"");text(p,title,full?24:249,UI.title+13,full?p.width-48:720,18,'fg',500);rule(p,full?0:225,UI.title+1+UI.toolbar,full?p.width:1095).name='Toolbar boundary';return p;}
const welcome=shell('macOS / welcome');text(welcome,'Taypeer',440,216,440,24,'fg',500);button(welcome,'Открыть базу…',440,264,156,'default',false,'folder');button(welcome,'Создать…',608,264,130,'default',false,'plus');text(welcome,'Недавние',440,344,440,12,'muted');[['Рабочая.taypeer','Внутренняя копия'],['Личная.taypeer','Внутренняя копия']].forEach(([s,d],i)=>{rule(welcome,440,376+i*60,440);icon(welcome,'database',448,395+i*60);text(welcome,s,480,389+i*60,320,14);text(welcome,d,480,411+i*60,320,11,'muted');});button(welcome,'Получить базу…',440,532,168);
const unlock=shell('macOS / unlock');icon(unlock,'lock-keyhole',452,226);text(unlock,'Рабочая.taypeer',480,222,400,20,'fg',500);field(unlock,'Мастер-пароль','••••••••••••',452,282,416,{secret:true,error:true});text(unlock,'Неверный пароль',612,326,256,12,'danger');button(unlock,'Touch ID',452,386,120,'default',false,'fingerprint');button(unlock,'Разблокировать',690,386,178,'primary');
const devices=utility('devices','Устройства');button(devices,'Пригласить…',1124,51,164,'default',false,'plus');['Устройство','Статус','Роль'].forEach((s,i)=>text(devices,s,[256,752,974][i],134,[480,210,190][i],12,'muted'));[['MacBook Pro','Это устройство','Управляющий'],['Pixel 9','В сети','Участник'],['Mac mini','Не в сети · 2 ч','Участник']].forEach(([a,b,c],i)=>{const y=166+i*48;rule(devices,256,y,1032);icon(devices,i===1?'smartphone':'monitor',256,y+16);text(devices,a,284,y+14,440,13);text(devices,b,752,y+14,200,13,'muted');text(devices,c,974,y+14,200,13,'muted');ib(devices,'ellipsis',1256,y+7);});rule(devices,256,340,1032);text(devices,'Синхронизация',256,384,800,16,'fg',500);text(devices,'Все изменения переданы · 09:41',256,426,700,13,'muted');button(devices,'Синхронизировать',1100,414,188,'default',false,'refresh-cw');
const settings=utility('settings','Настройки');
text(settings,'Устройство',249,100,128,13,'fg',500);text(settings,'База данных',401,100,144,13,'muted');rule(settings,225,127,1095);box(settings,'Selected tab / Устройство',225,126,152,2,'muted');
field(settings,'Имя устройства','MacBook Pro',249,152,608);field(settings,'Язык','Русский',249,200,608,{select:true});field(settings,'Автоблокировка','Через 5 минут',249,248,608,{select:true});field(settings,'Очистка буфера','Через 30 секунд',249,296,608,{select:true});toggle(settings,'Touch ID',249,364,608,true);toggle(settings,'Разрешить relay',249,412,608,true);
function dialog(name,title,h=420){const p=board('03 · Диалоги','Dialog / '+name,560,h);text(p,title,24,24,480,20,'fg',500);ib(p,'x',512,16);return p;}
const create=dialog('create','Создать базу',352);field(create,'Название','Рабочая',24,82,512);field(create,'Мастер-пароль','••••••••••••••',24,130,512,{secret:true});field(create,'Повтор пароля','••••••••••••••',24,178,512,{secret:true});button(create,'Параметры…',184,234,148);rule(create,0,284,560);button(create,'Отмена',316,302,96);button(create,'Создать',424,302,112,'primary');
const del=dialog('revoke','Отозвать доступ Pixel 9?',282);text(del,'Устройство потеряет доступ к новым изменениям.\nУже полученные данные останутся на нём.',24,82,512,14,'muted',400,52);button(del,'Отмена',294,216,104);button(del,'Отозвать',410,216,126,'danger');
const invite=dialog('invite','Подключить устройство',444);text(invite,'Код приглашения',24,82,512,12,'muted');text(invite,'8472  1963',24,122,440,28,'fg',500,40,true);ib(invite,'copy',496,124);text(invite,'Одноразовый · осталось 04:32',24,186,512,12,'muted');rule(invite,24,232,512);text(invite,'Pixel 9 запрашивает доступ',24,266,512,14,'fg',500);text(invite,'Код проверки: 742 618',24,300,512,14,'muted',400,24,true);button(invite,'Отклонить',286,378,120);button(invite,'Разрешить',418,378,118,'primary');
const generator=board('03 · Диалоги','Dialog / generator',560,440);
const generated=inputFrame(generator,'Generated password',24,24,416,36);
text(generated,'mT7!qR4#vN9@kL2$wH6%pD8&',12,9,360,14,'fg',400,20,true);ib(generated,'eye',380,2);
const regen=ib(generator,'dice-5',448,26);regen.name='Icon button / Сгенерировать заново';const copyGen=ib(generator,'copy',496,26);copyGen.name='Icon button / Копировать пароль';
box(generator,'Password quality / high',24,72,512,3,'success',1);
text(generator,'Качество: высокое',24,88,180,12,'muted');text(generator,'24 символа',208,88,144,12,'muted').textAlignHorizontal='CENTER';text(generator,'Энтропия: 157,3 бит',356,88,180,12,'muted').textAlignHorizontal='RIGHT';
text(generator,'Пароль',32,138,104,13,'fg',500);text(generator,'Парольная фраза',152,138,240,13,'muted');rule(generator,24,168,512);box(generator,'Selected tab',24,167,104,2,'fg');
text(generator,'Длина',24,194,120,12,'muted');box(generator,'Slider track',24,235,352,3,'border',1);box(generator,'Slider active track',24,235,68,3,'muted',1);box(generator,'Slider thumb',86,229,12,15,'fg',2);
const length=inputFrame(generator,'NumberInput / length',400,216,136,36);text(length,'24',12,9,52,13);box(length,'Decrement',78,17,12,1,'muted');icon(length,'plus',108,10);
text(generator,'Символы',24,276,512,12,'muted');
[['A–Z',96],['a–z',96],['0–9',96],['!@#',96]].forEach(([s,w],i)=>{const b=box(generator,'Toggle / checked / '+s,24+i*108,304,w,32,'selected',4,true);icon(b,'check',10,8);text(b,s,36,7,w-44,13);});
button(generator,'Дополнительно…',24,380,182);button(generator,'Закрыть',304,380,100);button(generator,'Использовать',416,380,120,'primary');
const error=dialog('save-error','Не удалось сохранить',276);text(error,'Недостаточно места. Изменения остались в форме.',24,82,512,14,'muted',400,44);button(error,'Вернуться',272,210,126);button(error,'Повторить',410,210,126,'primary');
// Isolated inspectors keep their own header so the shared edit scope remains visible.
function tabSlice(active,edit=false){const p=board('05 · Вкладки записи','Tabs / '+active,614,560);entryHeader(p,614,edit);tabs(p,0,UI.toolbar+1,614,active);return p;}
function attributesBody(p,top,edit=false){
 const w=614; text(p,'Атрибуты',24,top+16,360,16,'fg',500);
 if(edit)ib(p,'plus',558,top+10).name='Icon button / Добавить атрибут…';
 const head=top+52;
 [['Recovery code','••••••••••••',true],['Организация','Studio',false]].forEach(([key,val,secret],i)=>{
  const y=head+i*UI.row,n=box(p,'Attribute row / '+key,0,y,w,UI.row,'bg');
  text(n,key,24,12,144,13,'muted');
  text(n,val,184,12,edit?262:406,13).name='Attribute value / '+key;
  if(edit){
   if(secret)ib(n,'eye',454,6).name='Icon button / Показать атрибут / '+key;
   const c=box(n,'Checkbox / Защищённый атрибут / '+key,494,14,16,16,secret?'selected':'bg',3,true);if(secret)icon(c,'check',0,0);
   ib(n,'pencil',518,6).name='Icon button / Изменить атрибут / '+key;
   ib(n,'x',558,6).name='Icon button / Удалить атрибут / '+key;
  }else{
   if(secret){const a=ib(n,'eye',518,6);a.name='Icon button / Показать атрибут / '+key;a.visible=false;}
   const a=ib(n,'copy',558,6);a.name='Icon button / Копировать атрибут / '+key;a.visible=false;
  }
  rule(n,0,UI.row-1,w);
 });
 const attachment=head+2*UI.row;
 text(p,'Вложения',24,attachment+16,360,16,'fg',500);
 if(edit)ib(p,'file-plus-2',558,attachment+10).name='Icon button / Добавить вложение…';
 const y=attachment+52,n=box(p,'Attachment row / recovery-codes.txt',0,y,w,UI.row,'bg');
 icon(n,'file',24,14);text(n,'recovery-codes.txt',48,12,edit?390:502,13);
 if(edit){
  text(n,'20 KiB',446,12,64,12,'muted').textAlignHorizontal='RIGHT';
  ib(n,'pencil',518,6).name='Icon button / Переименовать вложение';
  ib(n,'x',558,6).name='Icon button / Удалить вложение';
  const hint=box(p,'Attachments hint',0,y+UI.row,w,64,'bg');
  text(hint,'Вложения сохраняются вместе с записью.',24,24,w-48,12,'muted');
 }else ib(n,'download',558,6).name='Icon button / Скачать вложение';
 rule(n,0,UI.row-1,w);
}
function appearanceBody(p,top,edit=false){
 const w=614;
 [['Иконка','key-round'],['Цвет текста','—'],['Цвет фона','—']].forEach(([label,val],i)=>{
  const y=top+i*UI.row,n=box(p,'Appearance row / '+label,0,y,w,UI.row,'bg');
  text(n,label,24,12,144,13,'muted');
  if(edit){
   if(i===0){icon(n,'key-round',184,14);button(n,'Выбрать иконку',208,6,160);ib(n,'image',376,6).name='Icon button / Изображение из файла';ib(n,'link',416,6).name='Icon button / Изображение по URL';}
   else{const f=inputFrame(n,'ColorPicker / '+label,184,6,160);text(f,val,12,6,112,13);icon(f,'chevron-down',132,8);ib(n,'x',352,6).name='Icon button / Сбросить '+label;}
  }else{
   if(i===0)icon(n,'key-round',184,14);else text(n,val,184,12,406,13);
  }
  rule(n,0,UI.row-1,w);
 });
 const previewTop=top+3*UI.row;
 text(p,'Предпросмотр',24,previewTop+16,566,16,'fg',500);
 const preview=box(p,'Appearance preview',24,previewTop+52,566,52,'bg',4);
 icon(preview,'key-round',16,18);text(preview,'GitHub',40,16,510,16);
}
const attributes=tabSlice('Дополнительно',true);attributesBody(attributes,83,true);
const appearance=tabSlice('Вид',true);appearanceBody(appearance,83,true);
const properties=tabSlice('Свойства');
[['Создание','2026-08-12 10:40:00.000'],['Изменение','2026-09-07 09:12:00.000'],['ID','e2194a38-679e-49f0-8fc2-a087bc8a9124']].forEach(([a,b],i)=>{
 const y=83+i*UI.row;box(properties,'Read-only property / '+a,0,y,614,UI.row,i%2?'panel':'bg');text(properties,a,24,y+12,144,13,'muted');text(properties,b,184,y+12,354,13);const copy=ib(properties,'copy',558,y+6);copy.name='Icon button / Копировать '+a;copy.visible=false;rule(properties,0,y+UI.row-1,614);
});
const history=tabSlice('История');text(history,'История',24,101,300,14,'fg',500);ib(history,'ellipsis',558,95).name='DropdownMenu / История / Очистить историю…';
['Сохранено','Название'].forEach((s,i)=>text(history,s,[24,248][i],149,[208,342][i],12,'muted'));rule(history,0,176,614);
[['2026-09-06 18:32:00.000','GitHub'],['2026-09-02 10:14:00.000','GitHub'],['2026-08-12 10:40:00.000','GitHub']].forEach(([date,title],i)=>{const y=177+i*UI.row;box(history,'History version / '+date,0,y,614,UI.row,i===0?'selected':i%2?'panel':'bg');text(history,date,24,y+12,208,13,'muted');text(history,title,248,y+12,342,13,'muted');rule(history,0,y+UI.row-1,614);});
button(history,'Просмотр',24,325,85);button(history,'Сравнить',117,325,96);button(history,'Восстановить',221,325,126);
// Design-system page: assets, field variants and compact read-only tab examples.
figma.currentPage=library;const kit=board('04 · Компоненты','System / components',1320,1536);text(kit,'Taypeer / GPUI',24,24,1000,24,'fg',500);text(kit,'Нейтральная тёмная тема · Lucide · medium / compact',24,64,1000,14,'muted');
let k=0;for(const [s,n] of Object.entries(icons)){kit.appendChild(n);const x=24+(k%9)*140,y=136+Math.floor(k/9)*76;n.x=x;n.y=y;text(kit,s,x,y+26,132,11,'muted');k++;}
text(kit,'Кнопки и состояния',24,534,1000,16,'fg',500);[['Обычная','default'],['Сохранить','primary'],['Удалить','danger'],['Недоступно','disabled']].forEach(([s,v],i)=>{const n=button(kit,s,24+i*172,576,156,v);figma.createComponentFromNode(n);});
const focus=button(kit,'Фокус',24,632,156);focus.strokes=paint('fg');focus.strokeWeight=2;figma.createComponentFromNode(focus);const hover=button(kit,'Наведение',196,632,156);fill(hover,'selected');figma.createComponentFromNode(hover);const pressed=button(kit,'Меню открыто',368,632,156);fill(pressed,'selected');figma.createComponentFromNode(pressed);const busy=button(kit,'Сохранение',540,632,156,'disabled');figma.createComponentFromNode(busy);
text(kit,'Поля / самостоятельная форма и строка записи',24,700,1000,16,'fg',500);
field(kit,'Название','Рабочая',24,744,392);field(kit,'Пустое поле','',448,744,392);field(kit,'Ошибка','',872,744,392,{error:true});text(kit,'Введите название',1032,784,232,12,'danger');
field(kit,'Фокус','Рабочая',24,824,392,{state:'focus'});field(kit,'Строка записи','GitHub',448,824,392,{variant:'row'});rule(kit,448,863,392);toggle(kit,'Сразу применяется',872,832,392,true);
text(kit,'Сортировка',24,894,144,13,'muted');icon(kit,'arrow-up',184,896);icon(kit,'arrow-down',224,896);icon(kit,'chevrons-up-down',264,896);
text(kit,'Просмотр вкладок · тот же объект и общая область редактирования',24,958,1240,16,'fg',500);
const attrView=box(kit,'Component / Дополнительно / просмотр',24,1000,614,500,'bg',4,true);entryHeader(attrView,614);tabs(attrView,0,45,614,'Дополнительно');attributesBody(attrView,83,false);figma.createComponentFromNode(attrView);
const appearanceView=box(kit,'Component / Вид / просмотр',662,1000,614,500,'bg',4,true);entryHeader(appearanceView,614);tabs(appearanceView,0,45,614,'Вид');appearanceBody(appearanceView,83,false);figma.createComponentFromNode(appearanceView);
// Overview stays outside product screens; annotations are not UI help.
const overview=board('00 · Начать здесь','Taypeer / обзор',1120,760);text(overview,'Taypeer',32,32,900,32,'fg',500);text(overview,'Компактный desktop-интерфейс',32,84,1000,18,'muted');rule(overview,32,138,1056);[['01 / macOS','12 экранов · группы, записи, редактор, обмен и настройки'],['02 / Android','8 экранов · отдельные шаги вместо трёх панелей'],['03 / Диалоги','6 решений · создание, изменения, доступ, генератор и ошибка'],['04 / Компоненты','37 Lucide-ассетов · состояния контролов · цветовые переменные']].forEach(([a,b],i)=>{const y=174+i*100;text(overview,a,32,y,1000,18,'fg',500);text(overview,b,32,y+34,1000,14,'muted');});rule(overview,32,590,1056);text(overview,'Основной путь',32,622,1000,14,'fg',500);text(overview,'Открыть базу → группа → запись → изменить → сохранить',32,656,1000,14,'muted');text(overview,'Редактируемые макеты. Поведение и клавиатура описаны в README и spec.md.',32,708,1000,12,'muted');
// Resting input geometry and paint are owned by inputFrame; no global overrides.
buildDesktopV1();
buildAndroidV1();
return {boards:boards.length,counts,icons:Object.keys(icons).length,nodes:figma.root.findAll(()=>true).length};
