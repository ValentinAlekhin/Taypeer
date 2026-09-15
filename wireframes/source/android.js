// Phone compositions only. Shared paint, font primitives and licensed vectors
// come from compact.js; desktop components retain their original geometry.
function buildAndroidV1() {
  const MAIN='02 · Android', FORMS='09 · Android · Формы', STATES='10 · Android · Меню и состояния', QA='11 · Android · Тема и размеры', FLOW='12 · Android · Карта сценариев';
  for(const name of [FORMS,STATES,QA,FLOW]) pages[name]=Object.assign(figma.createPage(),{name});
  const made=[], mobileIcons={};
  const kit=board(STATES,'Android states / components',1260,1040);
  text(kit,'Android · компоненты телефона',24,24,1200,26,'fg',500,40);
  text(kit,'Lucide 20 · линия 1,75 · область действия 48 × 48 dp · текст 16 / подписи 14',24,74,1200,16);
  Object.entries(icons).forEach(([key,original],i)=>{
    const n=original.clone();kit.appendChild(n);mobileIcons[key]=n;n.name='Mobile icon / '+key;
    n.resize(20,20);n.x=32+i%12*100;n.y=134+Math.floor(i/12)*96;
    for(const v of n.findAll(c=>c.type==='VECTOR')) {
      v.x*=1.25;v.y*=1.25;
      v.vectorPaths=v.vectorPaths.map(p=>({...p,data:p.data.replace(/-?\d*\.?\d+(?:e[-+]?\d+)?/gi,s=>String(Number(s)*1.25))}));
      v.strokeWeight=1.75;
    }
    text(kit,key,n.x,n.y+28,90,14,'muted',400,48);
  });
  function glyph(p,key,x,y) {
    if(key==='settings')key='sliders-horizontal';
    const n=mobileIcons[key].createInstance();p.appendChild(n);n.name='Mobile glyph / '+key;n.x=x;n.y=y;return n;
  }
  function copy(p,s,x,y,w,{size=16,color='fg',weight=400,h}={}) {
    // Reserve for word wrapping as well as explicit newlines. Counting only
    // total characters underestimated Cyrillic paragraphs with long words.
    const lines=s.split('\n').reduce((total,line)=>{
      let used=0,rows=1;
      for(const word of line.split(' ')){const width=word.length*size*0.64;if(used&&used+width>w){rows++;used=0;}rows+=Math.max(0,Math.ceil(width/w)-1);used=(width>w?width%w:used+width)+size*0.3;}
      return total+rows;
    },0);
    return text(p,s,x,y,w,size,color,weight,h||Math.ceil(size*1.5)*lines);
  }
  function touch(p,label,x,y,w=48,h=48,kind='ghost') {
    return box(p,'Touch / '+kind+' / '+label,x,y,w,h,kind==='primary'?'primary':kind==='default'?'chrome':kind==='selected'?'selected':kind==='disabled'?'panel':'',6);
  }
  function action(p,label,x,y,w,kind='default',h=48) {
    const n=touch(p,label,x,y,w,h,kind);
    const select=label.endsWith(' ▾');
    copy(n,select?label.slice(0,-2):label,12,(h-24)/2,w-(select?64:24),{color:kind==='primary'?'onPrimary':kind==='danger'?'danger':kind==='disabled'?'dim':'fg',weight:500,h:24});
    if(select)glyph(n,'chevron-down',w-34,(h-20)/2);return n;
  }
  function iconAction(p,key,label,x,y) {const n=touch(p,label,x,y);glyph(n,key,14,14);return n;}
  function paragraph(c,s,color='muted',size=14) {const n=copy(c.body,s,16,c.y,c.w-32,{size,color});c.y+=n.height+16;return n;}
  function heading(c,s) {return paragraph(c,s,'fg',18);}
  function fullAction(c,s,kind='default') {const n=action(c.body,s,16,c.y,c.w-32,kind);c.y+=60;return n;}
  function separator(c) {rule(c.body,16,c.y,c.w-32);c.y+=20;}
  function field(c,label,value,{secret=false,select=false,edit=true,entry=false,focus=false,lines=1,actions=[]}={}) {
    const labelNode=copy(c.body,label,16,c.y,c.w-32,{size:14,color:'muted'});c.y+=labelNode.height+4;
    const h=Math.max(48,lines*24+16), w=c.w-32;
    const n=inputFrame(c.body,label,16,c.y,w,h,entry?'row':'standalone',focus?'focus':'rest');
    const commands=actions.length?actions:secret?[['eye','Показать '+label]]:select?[['chevron-down','Выбрать '+label]]:[];
    const remaining=w-commands.length*48;
    const valueParent=edit?touch(n,'Ввести '+label,0,0,remaining,h):n;
    const valueText=copy(valueParent,value,entry?0:12,(h-lines*24)/2,remaining-(entry?8:24),{h:lines*24});
    valueText.name='Value / '+label;
    commands.forEach(([key,name],i)=>iconAction(n,key,name,remaining+i*48,0));
    c.y+=h+16;
    if(entry)rule(c.body,16,c.y-8,w);
    return n;
  }
  function choice(c,label,selected=false,{radio=false}={}) {
    const n=touch(c.body,label,16,c.y,c.w-32,48,selected?'selected':'ghost');
    if(radio){const dot=figma.createEllipse();n.appendChild(dot);dot.x=14;dot.y=14;dot.resize(20,20);dot.fills=[];dot.strokes=paint('muted');dot.strokeWeight=1;if(selected){const inner=box(n,'Selected radio',19,19,10,10,'fg',5);}}
    else {box(n,'Checkbox',14,14,20,20,selected?'selected':'',4,true);if(selected)glyph(n,'check',14,14);}
    copy(n,label,48,12,n.width-60,{h:24});c.y+=52;return n;
  }
  function item(c,title,subtitle='',key='key-round',{selected=false,end='chevron-right'}={}) {
    const tw=c.w-112;
    const th=Math.max(24,Math.ceil(title.length*9/tw)*24),sh=subtitle?Math.ceil(subtitle.length*8/tw)*21:0;
    const h=Math.max(64,th+sh+24);
    const n=touch(c.body,title,8,c.y,c.w-16,h,selected?'selected':'ghost');
    glyph(n,key,16,20);copy(n,title,52,12,tw,{weight:500,h:th});
    if(subtitle)copy(n,subtitle,52,12+th,tw,{size:14,color:'muted',h:sh});
    if(end)glyph(n,end,n.width-32,22);
    rule(n,52,h-1,n.width-68);c.y+=h;return n;
  }
  function notice(c,title,summary,{tone='warning',button}={}) {
    const start=c.y;c.y+=12;const host=c.body;
    const titleText=copy(host,title,28,c.y,c.w-56,{color:tone,weight:500});c.y+=titleText.height+8;
    const message=copy(host,summary,28,c.y,c.w-56,{size:14,color:'muted'});c.y+=message.height+12;
    if(button){action(host,button,28,c.y,c.w-56,'ghost');c.y+=56;}
    rule(host,16,start,c.w-32);rule(host,16,c.y,c.w-32);c.y+=20;
  }
  function ime(p,top) {
    const n=box(p,'System / IME illustration',0,top,p.width,p.height-top-24,'chrome');
    copy(n,'Русский',16,8,n.width-32,{size:14,color:'muted',h:24});
    const rows=['й ц у к е н г ш щ з','ф ы в а п р о л д ж','я ч с м и т ь б ю'];
    const compact=n.height<250;
    rows.forEach((row,i)=>{const keys=row.split(' '),kw=(n.width-16)/keys.length;keys.forEach((s,j)=>{
      const k=box(n,'System key',8+j*kw,(compact?30:42)+i*(compact?26:48),kw-4,compact?24:42,'raised',4);copy(k,s,8,compact?0:8,kw-16,{size:compact?14:16,h:compact?22:24});
    });});
    if(n.height>=250){box(n,'System space',76,192,n.width-152,42,'raised',4);copy(n,'Пробел',n.width/2-42,202,84,{size:14,h:24});}
  }
  function phone(name,title,{page=MAIN,w=390,h=844,menu=false,right='ellipsis',rightLabel='Действия',form,keyboard=false,tab,tabOffset=0}={}) {
    const p=board(page,'Android'+(page===FORMS?' form':page===QA?' QA':'')+' / '+name,w,h);p.cornerRadius=0;made.push(p);
    const landscape=w>h, inset=landscape?32:0,inner=w-inset*2;
    const status=box(p,'System / status',0,0,w,32,'bg');copy(status,'9:41',inset+16,5,80,{size:14,h:22});copy(status,'Wi-Fi  100%',w-inset-116,5,100,{size:14,color:'muted',h:22});
    if(!landscape)box(status,'System / camera cutout',w/2-5,10,10,10,'border',5);
    const bar=box(p,'Android / app bar',inset,32,inner,56,'bg');
    if(menu){const m=touch(bar,'Открыть меню',8,4);for(let i=0;i<3;i++)box(m,'Menu line',14,17+i*6,20,1.5,'muted');}
    else iconAction(bar,'arrow-left','Назад',8,4);
    copy(bar,title,64,title.length>24?0:14,inner-(right?124:80),{size:18,weight:500,h:title.length>24?54:28});
    if(right)iconAction(bar,right,rightLabel,inner-56,4);
    rule(p,inset,87,inner);
    const bottom=keyboard?h-24-(landscape?112:280):h-24;
    if(keyboard)ime(p,bottom);
    if(form){const f=box(p,'Android / form actions',inset,bottom-64,inner,64,'panel');rule(f,0,0,inner);const cw=Math.floor((inner-40)/2);action(f,form.cancel||'Отмена',16,8,cw,'ghost');action(f,form.label,24+cw,8,cw,form.kind||'primary');}
    const top=88+(tab?56:0), end=bottom-(form?64:0);
    const viewport=box(p,'Android / scroll viewport',inset,top,inner,end-top,'bg');viewport.clipsContent=true;
    const body=box(viewport,'Android / scroll content',0,0,inner,2000,'');
    const c={p,body,viewport,w:inner,y:16};
    if(tab)tabsFor(p,inset,88,inner,tab,tabOffset);
    const nav=box(p,'System / navigation',0,h-24,w,24,'bg');box(nav,'System / home indicator',w/2-48,10,96,4,'muted',2);
    return c;
  }
  function tabsFor(p,x,y,w,active,offset=0) {
    const v=box(p,'Android / tabs viewport',x,y,w,56,'bg');v.clipsContent=true;
    const t=box(v,'Android / tab content',-offset,0,598,56,'');
    let dx=8;
    [['Обзор',92],['Дополнительно',160],['Вид',72],['Свойства',124],['История',126]].forEach(([s,width])=>{
      const n=touch(t,'Вкладка '+s,dx,0,width,55);copy(n,s,12,16,width-24,{color:s===active?'fg':'muted',weight:500,h:24});
      if(s===active)box(n,'Android / active tab',12,53,width-24,2,'fg');dx+=width;
    });rule(v,0,55,w);
  }
  function finish(c,{scroll=0}={}) {
    c.body.resize(c.w,Math.max(c.y+16,c.viewport.height));
    c.body.y=-Math.min(scroll,Math.max(0,c.body.height-c.viewport.height));
    if(c.body.height>c.viewport.height)box(c.p,'Android / scroll thumb',c.viewport.x+c.w-4,c.viewport.y+12,2,Math.max(30,c.viewport.height*c.viewport.height/c.body.height),'dim',1);
    return c.p;
  }
  function form(name,title,render,{label='Сохранить',kind='primary',...options}={}) {
    const c=phone(name,title,{page:FORMS,right:null,form:{label,kind},...options});render(c);return finish(c);
  }
  function confirm(name,title,summary,label,kind='primary') {return form(name,title,c=>paragraph(c,summary,'fg',16),{label,kind});}
  function search(c,query='Поиск записей…') {const f=inputFrame(c.body,'Поиск',16,c.y,c.w-32,48);const n=touch(f,'Искать записи',0,0,f.width,48);glyph(n,'search',14,14);copy(n,query,48,12,f.width-64,{color:query.includes('…')?'muted':'fg',h:24});c.y+=60;}

  function entry(name,{edit=false,tab='Обзор',newEntry=false,...options}={}) {
    const c=phone(name,newEntry?'Новая запись':'GitHub',{right:edit?null:'pencil',rightLabel:'Редактировать запись',form:edit?{label:newEntry?'Создать':'Сохранить'}:null,tab,tabOffset:['Свойства','История'].includes(tab)?216:tab==='Вид'?100:0,...options});
    if(tab==='Обзор') {
      const row={entry:true,edit};
      field(c,'Название',newEntry?'Новая запись':'GitHub',row);
      field(c,'Логин',newEntry?'':'demo@studio.example',{...row,actions:edit?[]:[['copy','Копировать логин']]});
      field(c,'Пароль','••••••••••••',{...row,actions:edit?[['eye','Показать пароль'],['dice-5','Генератор пароля']]:[['eye','Показать пароль'],['copy','Копировать пароль']]});
      field(c,'URL','https://github.com',{...row,actions:edit?[['download','Загрузить favicon…']]:[['copy','Копировать URL']]});
      field(c,'Теги','работа, разработка',row);
      field(c,'Истекает','Бессрочно',{...row,actions:edit?[['chevron-down','Выбрать срок…']]:[]});
      field(c,'Заметки','Основной рабочий аккаунт.\nИскусственные данные макета.',{...row,lines:3});
    } else if(tab==='Дополнительно') {
      heading(c,'Атрибуты');
      field(c,'Recovery code','••••••••••••',{entry:true,edit:false,actions:[['eye','Показать атрибут'],['copy','Копировать атрибут']]});
      if(edit)fullAction(c,'Изменить атрибут…');
      field(c,'Организация','Studio',{entry:true,edit:false,actions:edit?[['ellipsis','Действия атрибута…']]:[['copy','Копировать организацию']]});
      if(edit)fullAction(c,'Добавить атрибут…');
      heading(c,'Вложения');item(c,'recovery-codes.txt','20 КиБ · получено','file',{end:'download'});
      if(edit){fullAction(c,'Действия вложения…');fullAction(c,'Добавить вложение…');}
      paragraph(c,'2,4 / 100 МиБ · на файл до 10 МиБ');
    } else if(tab==='Вид') {
      heading(c,'Оформление');item(c,'GitHub','Предпросмотр записи','key-round',{end:null});
      if(edit){fullAction(c,'Выбрать значок…');fullAction(c,'Загрузить по URL…');fullAction(c,'Выбрать файл…');}
      field(c,'Основной цвет','По умолчанию',{edit:false,select:edit});field(c,'Цвет фона','По умолчанию',{edit:false,select:edit});
      if(edit)fullAction(c,'Сбросить оформление');
    } else if(tab==='Свойства') {
      for(const [label,value] of [['Создание','5 сентября 2026, 12:10'],['Изменение','15 сентября 2026, 09:32'],['Доступ на этом устройстве','15 сентября 2026, 09:41']])field(c,label,value,{edit:false,entry:true,actions:[['copy','Копировать '+label]]});
      field(c,'ID','00000000-0000-4000-\n8000-000000000042',{edit:false,entry:true,lines:2,actions:[['copy','Копировать ID']]});paragraph(c,'Доступ хранится только на этом устройстве.');
    } else {
      heading(c,'Сохранённые версии');
      item(c,'Сегодня, 09:32','Pixel 9 · изменён пароль','history',{selected:true});item(c,'Вчера, 18:20','MacBook Pro · добавлен URL','history');item(c,'5 сентября, 12:10','MacBook Pro · создание','history');
      c.y+=16;fullAction(c,'Просмотреть версию…');fullAction(c,'Сравнить с текущей…');fullAction(c,'Восстановить версию…');fullAction(c,'Очистить историю…','danger');
    }
    return finish(c);
  }
  function entriesScreen(name='entries',options={}) {
    const c=phone(name,options.long?'Инфраструктура и сервисы':'Работа',{menu:true,...options});search(c);
    c.viewport.resize(c.w,c.p.height-108-c.viewport.y);
    if(options.conflict)notice(c,'3 конфликта','Выберите итог для спорных правок.',{button:'Разобрать…'});
    for(const [s,v] of entries)item(c,s,v.replace('valentin','demo').replace('studio.dev','studio.example'));
    const p=finish(c);const fab=touch(p,'Создать запись…',p.width-80,p.height-96,56,56,'default');glyph(fab,'plus',18,18);return p;
  }
  const welcome=phone('welcome','Taypeer',{right:'settings',rightLabel:'Настройки'});
  welcome.y=40;heading(welcome,'Ваши пароли.\nНа ваших устройствах.');paragraph(welcome,'Создайте базу или откройте имеющуюся.');
  fullAction(welcome,'Создать базу…');fullAction(welcome,'Открыть .taypeer…');fullAction(welcome,'Получить с устройства…');fullAction(welcome,'Импортировать KDBX…');separator(welcome);heading(welcome,'Последние базы');
  item(welcome,'Рабочая','Внутренняя копия · заблокирована','database');item(welcome,'Личная','Внутренняя копия · вчера','database');finish(welcome);
  const unlock=phone('unlock','Рабочая',{right:'settings',rightLabel:'Настройки'});unlock.y=64;glyph(unlock.body,'lock-keyhole',16,24);heading(unlock,'База заблокирована');paragraph(unlock,'Внутренняя рабочая копия');field(unlock,'Мастер-пароль','••••••••••••',{secret:true});fullAction(unlock,'Разблокировать','primary');fullAction(unlock,'Биометрия…');paragraph(unlock,'Подготовка ключа: 1,2 с на Pixel 9');notice(unlock,'Изменения получены','Применение после разблокировки.',{tone:'muted'});fullAction(unlock,'Выбрать другую базу…');finish(unlock);
  entriesScreen();entriesScreen('entries-conflicts',{conflict:true});
  const empty=phone('empty','Рабочая',{menu:true});search(empty);empty.y+=64;heading(empty,'Пока нет групп');paragraph(empty,'Создайте первую группу для записей.');fullAction(empty,'Создать группу…');finish(empty);
  const emptyGroup=phone('empty-group','Архив',{menu:true});search(emptyGroup);emptyGroup.y+=64;heading(emptyGroup,'В группе пока нет записей');fullAction(emptyGroup,'Создать запись…');finish(emptyGroup);
  function drawer(name,scroll=0,options={}) {
    const c=phone(name,'Работа',{menu:true,...options});c.viewport.remove();
    const scrim=box(c.p,'Android / drawer scrim',0,32,c.p.width,c.p.height-56,'#000000');scrim.opacity=0.65;
    const w=Math.min(c.p.width-48,342),v=box(c.p,'Android / drawer viewport',0,32,w,c.p.height-56,'panel');v.clipsContent=true;
    c.viewport=v;c.w=w;c.body=box(v,'Android / scroll content',0,0,w,1400,'panel');c.y=8;
    item(c,'Рабочая','Внутренняя копия','database',{end:'chevron-down'});separator(c);
    [['Работа',0,true],['Сервисы',1,false],['Инфраструктура',1,false],['Личное',0,false],['Архив',0,false]].forEach(([name,depth,selected])=>{
      const row=box(c.body,'Android / group row',8,c.y,w-16,48,selected?'selected':'',4);
      iconAction(row,depth?'chevron-right':'chevron-down','Раскрыть '+name,depth*16,0);
      const label=touch(row,'Открыть группу '+name,48+depth*16,0,row.width-48-depth*16,48);glyph(label,'folder',8,14);copy(label,name,36,12,label.width-44,{h:24});c.y+=48;
    });
    fullAction(c,'Создать группу…');separator(c);
    for(const [s,key] of [['Устройства и обмен','monitor'],['Корзина','trash-2'],['Конфликты · 3','triangle-alert'],['Отложенные правки · 2','history'],['Импорт и экспорт','file'],['Настройки','settings'],['Заблокировать','lock-keyhole']])item(c,s,'',key,{end:null});
    return finish(c,{scroll});
  }
  drawer('drawer');drawer('drawer-scrolled',300);
  for(const mode of ['current','all','empty']) {
    const c=phone('search-'+mode,'Поиск',{right:'x',rightLabel:'Очистить поиск'});search(c,mode==='empty'?'несуществующая запись':'studio');fullAction(c,mode==='all'?'Все разблокированные БД ▾':'Текущая БД: Рабочая ▾');
    if(mode==='empty'){c.y+=36;heading(c,'Ничего не найдено');paragraph(c,'Измените запрос или область поиска.');}
    else {item(c,'Figma','demo@studio.example\nРабочая / Работа');item(c,'Cloudflare','admin@studio.example\nРабочая / Инфраструктура');if(mode==='all')item(c,'Studio Forum','demo@studio.example\nЛичная / Сервисы');}
    finish(c);
  }
  entry('entry');entry('edit',{edit:true});entry('new-entry',{edit:true,newEntry:true});
  for(const [tab,id] of [['Дополнительно','advanced'],['Вид','appearance'],['Свойства','properties'],['История','history']])entry(id,{tab});
  entry('advanced-edit',{edit:true,tab:'Дополнительно'});entry('appearance-edit',{edit:true,tab:'Вид'});
  const version=phone('history-version','Версия · 15 сен, 09:32',{tab:'Обзор',right:'ellipsis'});paragraph(version,'GitHub · сохранённая версия\nPixel 9 · только просмотр');field(version,'Название','GitHub',{edit:false,entry:true});field(version,'Логин','demo@studio.example',{edit:false,entry:true});field(version,'Пароль','••••••••••••',{edit:false,entry:true,actions:[['eye','Показать исторический пароль'],['copy','Копировать исторический пароль']]});fullAction(version,'Сравнить с текущей…');fullAction(version,'Восстановить…');finish(version);
  const compare=phone('history-compare','Сравнение версий',{right:null});paragraph(compare,'GitHub · изменения по полям');heading(compare,'Пароль');paragraph(compare,'Выбранная · 15 сентября, 09:32');field(compare,'Сохранённый пароль','••••••••••••',{edit:false,secret:true,actions:[['eye','Показать прежний пароль'],['copy','Копировать прежний пароль']]});paragraph(compare,'Текущая · 15 сентября, 09:41');field(compare,'Текущий пароль','••••••••••••••',{edit:false,secret:true,actions:[['eye','Показать текущий пароль'],['copy','Копировать текущий пароль']]});separator(compare);heading(compare,'Заметки');paragraph(compare,'Выбранная\nОсновной аккаунт\n\nТекущая\nОсновной рабочий аккаунт','fg',16);fullAction(compare,'Восстановить выбранную…');finish(compare);
  const trash=phone('trash','Корзина');item(trash,'Старый сервер','Запись · удалена вчера','file',{selected:true});item(trash,'Архив проектов','Группа · 4 записи','folder');trash.y+=24;fullAction(trash,'Восстановить…');fullAction(trash,'Удалить окончательно…','danger');fullAction(trash,'Очистить корзину…','danger');paragraph(trash,'Окончательная очистка не удаляет копии на других устройствах.');finish(trash);
  const devices=phone('devices','Устройства и обмен');paragraph(devices,'Рабочая · это устройство управляет БД');item(devices,'Pixel 9','Это устройство · управляющее','smartphone',{end:'ellipsis'});item(devices,'MacBook Pro','В сети · доверенное','monitor',{end:'ellipsis'});item(devices,'Mac mini','Не в сети · 2 часа','monitor',{end:'ellipsis'});devices.y+=16;fullAction(devices,'Пригласить устройство…');heading(devices,'Обмен');paragraph(devices,'Изменения применены · 09:41\nПрямое соединение');fullAction(devices,'Синхронизировать');finish(devices);
  const receive=phone('receive','Получить базу',{right:null,form:{label:'Подключиться'}});field(receive,'Код приглашения',DEMO_INVITATION.code.match(/.{1,28}/g).join('\n'),{lines:4});fullAction(receive,'Сканировать QR…');paragraph(receive,'Управляющее устройство подтвердит подключение. База сохранится во внутреннем хранилище.');finish(receive);
  const download=phone('receiving','Получение базы',{right:null});heading(download,'Загрузка зашифрованной БД');paragraph(download,'MacBook Pro → Pixel 9');box(download.body,'Android / transfer track',16,download.y,358,4,'border');box(download.body,'Android / transfer progress',16,download.y,220,4,'muted');download.y+=28;paragraph(download,'1,5 из 2,4 МиБ · 62%','fg',16);fullAction(download,'Отменить');notice(download,'Применение после входа','Данные станут доступны после завершения загрузки и ввода мастер-пароля.',{tone:'muted'});finish(download);
  const conflicts=phone('conflicts','Конфликты · 3');paragraph(conflicts,'Требуется ваш выбор. Варианты сохранены.');item(conflicts,'GitHub · пароль','2 варианта значения','triangle-alert');item(conflicts,'Старый сервер','Удаление и параллельная правка','trash-2');item(conflicts,'Инфраструктура','Разные группы назначения','folder');finish(conflicts);
  const resolver=phone('conflict','GitHub · пароль',{right:null,form:{label:'Применить',kind:'disabled'}});paragraph(resolver,'Выберите вариант. Общая копия пароля недоступна до решения.');
  for(const name of ['MacBook Pro · 09:32','Pixel 9 · 09:35']) {choice(resolver,name,false,{radio:true});field(resolver,'Вариант · '+name,'••••••••••••',{edit:false,entry:true,actions:[['eye','Показать вариант'],['copy','Копировать вариант']]});}
  choice(resolver,'Своё значение',false,{radio:true});paragraph(resolver,'Оба исходных варианта останутся в истории.');finish(resolver);
  const pending=phone('pending','Отложенные правки');item(pending,'Старый сервер','Объект окончательно очищен','history');item(pending,'Правки Mac mini','Нужен разбор старой схемы','history');paragraph(pending,'Остальные допустимые правки применяются. Источники сохраняются до отдельного решения.');finish(pending);
  const source=phone('pending-source','Отложенный источник',{right:null});heading(source,'Старый сервер');paragraph(source,'Mac mini · 12 сентября\nОбъект уже очищен. Автоматически он не восстановлен.');field(source,'Логин','demo',{edit:false});field(source,'Пароль','••••••••••••',{edit:false,actions:[['eye','Показать источник'],['copy','Копировать источник']]});fullAction(source,'Извлечь в новую запись…');fullAction(source,'Удалить источник…','danger');paragraph(source,'Извлечение сохраняет происхождение правок и не возвращает сетевой допуск.');finish(source);
  function settings(name='settings',options={}) {const c=phone(name,'Настройки',{right:null,...options});fullAction(c,'Устройство ▾');field(c,'Имя устройства','Pixel 9');field(c,'Язык','Русский',{select:true,edit:false});field(c,'Тема','Системная',{select:true,edit:false});field(c,'Автоблокировка','Через 5 минут',{select:true,edit:false});field(c,'Очистка буфера','Через 30 секунд',{select:true,edit:false});if(!options.locked)choice(c,'Биометрия · Рабочая',true);else paragraph(c,'Разблокируйте базу для настройки биометрии.');field(c,'Relay','Общие серверы',{select:true,edit:false});fullAction(c,'Собственный relay…');return finish(c);}
  settings();settings('settings-locked',{locked:true});
  const dbSettings=phone('settings-database','Настройки',{right:null});fullAction(dbSettings,'База данных ▾');heading(dbSettings,'Рабочая');paragraph(dbSettings,'Внутренняя копия · Pixel 9 управляет БД');fullAction(dbSettings,'Название и описание…');fullAction(dbSettings,'Сменить мастер-пароль…');fullAction(dbSettings,'Параметры защиты…');fullAction(dbSettings,'Лимиты вложений…');fullAction(dbSettings,'Устройства и обмен…');fullAction(dbSettings,'Обновить формат…');fullAction(dbSettings,'Восстановить управление…');finish(dbSettings);
  const files=phone('files','Импорт и экспорт',{right:null});paragraph(files,'Рабочая · внутренняя копия');fullAction(files,'Импортировать KDBX…');paragraph(files,'Создать отдельную БД Taypeer.');fullAction(files,'Экспортировать KDBX…');paragraph(files,'KDBX 4.1 с отдельным паролем.');fullAction(files,'Сохранить копию .taypeer…');paragraph(files,'Выберите место системным диалогом. Исходный файл импорта не обновляется.');finish(files);

  // Long forms are routes; short decisions and parameter choices are sheets.
  form('create','Создать базу',c=>{field(c,'Название','Рабочая');field(c,'Мастер-пароль','••••••••••••',{secret:true});field(c,'Повтор пароля','••••••••••••',{secret:true});paragraph(c,'Качество: надёжный');fullAction(c,'Параметры защиты…');paragraph(c,'Рабочая копия будет во внутреннем хранилище. Этот телефон станет управляющим устройством.');},{label:'Создать'});
  form('group','Создать группу',c=>{field(c,'Название','Инфраструктура');field(c,'Описание','Серверы и рабочие сервисы',{lines:3});field(c,'Родитель','Верхний уровень',{select:true,edit:false});fullAction(c,'Выбрать значок…');},{label:'Создать'});
  form('attribute','Изменить атрибут',c=>{field(c,'Название','Recovery code');field(c,'Значение','••••••••••••',{secret:true,lines:3});choice(c,'Защищённый атрибут',true);paragraph(c,'Скрыт по умолчанию и исключён из поиска.');});
  form('attachment','Переименовать вложение',c=>{paragraph(c,'recovery-codes.txt · 20 КиБ');field(c,'Новое имя','recovery-codes-work.txt');});
  form('destination','Выбрать группу',c=>{notice(c,'Исходной группы больше нет','Выберите существующую или создайте новую.');item(c,'Работа','Рабочая / Работа','folder');item(c,'Личное','Рабочая / Личное','folder');fullAction(c,'Создать группу…');},{label:'Восстановить',kind:'disabled'});
  form('move','Переместить запись',c=>{paragraph(c,'GitHub · текущая группа: Работа');item(c,'Работа','','folder');item(c,'Сервисы','Рабочая / Работа / Сервисы','folder',{selected:true});item(c,'Личное','','folder');},{label:'Переместить'});
  form('full-value','URL целиком',c=>{paragraph(c,'https://service.example/teams/\ninfrastructure/production/\ncredentials?environment=preview\n&workspace=studio-demo','fg',16);},{label:'Копировать'});
  function generator(name,phrase=false) {return form(name,'',c=>{field(c,'Результат','••••••••••••••••',{edit:false,actions:[['eye','Показать результат'],['dice-5','Сгенерировать заново'],['copy','Копировать результат']]});box(c.body,'Android / strength',16,c.y,c.w-32,4,'success');c.y+=16;paragraph(c,phrase?'Надёжный · 6 слов · 77,55 бит':'Надёжный · 30 символов · 196,64 бит','fg',14);fullAction(c,phrase?'Парольная фраза ▾':'Пароль ▾');if(phrase){field(c,'Количество слов · 3–20','6');field(c,'Разделитель','-');paragraph(c,'EFF Long Wordlist · 7776 слов\nАнглийский словарь доступен без сети.');}else{field(c,'Длина · 1–256','30');const slider=touch(c.body,'Длина пароля',16,c.y,c.w-32,48);box(slider,'Slider track',12,23,slider.width-24,2,'border');box(slider,'Slider thumb',44,16,16,16,'fg',8);c.y+=56;choice(c,'A–Z',true);choice(c,'a–z',true);choice(c,'0–9',true);choice(c,'Спецсимволы',true);fullAction(c,'Исключения…');}},{label:'Использовать',form:{label:'Использовать',cancel:'Закрыть'}});}
  generator('generator');generator('passphrase',true);
  form('generator-exclusions','Исключения',c=>{choice(c,'Исключать похожие символы',true);field(c,'Исключить символы','O0Il1');paragraph(c,'Изменение параметров создаёт новый результат.');},{label:'Применить'});
  form('image-url','Загрузить значок',c=>{field(c,'URL изображения','https://studio.example/icon.png',{lines:2});paragraph(c,'Запрос выполняется только после «Загрузить». Изображение до 1 МиБ.');},{label:'Загрузить'});
  form('bulk-icons','Загрузить значки',c=>{paragraph(c,'Группа: Работа');choice(c,'Включая подгруппы');choice(c,'Заменять существующие');paragraph(c,'По умолчанию — только записи этой группы без собственных значков.');},{label:'Загрузить'});
  form('protection','Параметры защиты',c=>{paragraph(c,'Рабочая · управляющее устройство');field(c,'Целевое время подготовки ключа','1 секунда',{select:true,edit:false});field(c,'Память','64 МиБ',{select:true,edit:false});paragraph(c,'Argon2id · диапазон времени 0,5–5 с\nФактическая подготовка на Pixel 9: 1,2 с.');fullAction(c,'Калибровать');},{label:'Применить'});
  form('limits','Лимиты вложений',c=>{field(c,'Размер одного файла, МиБ','10');field(c,'Всего вложений, МиБ','100');paragraph(c,'Занято 2,4 МиБ. История, корзина и ожидающие источники могут удерживать содержимое.');},{label:'Применить'});
  for(const revoke of [false,true])form(revoke?'revoke':'change-password',revoke?'Отозвать Mac mini':'Сменить мастер-пароль',c=>{field(c,'Новый пароль','••••••••••••',{secret:true});field(c,'Повтор пароля','••••••••••••',{secret:true});paragraph(c,'Качество: надёжный');paragraph(c,revoke?'Mac mini потеряет допуск после получения нового состояния. Уже полученные данные и старый пароль останутся на его копиях.':'Другие устройства после получения нового состояния потребуют новый пароль.');},{label:revoke?'Отозвать':'Сменить',kind:revoke?'danger':'primary'});
  form('recover-control','Восстановить управление',c=>{paragraph(c,'Будет создана новая рабочая копия с новым доверенным набором. Этот Pixel 9 станет управляющим.');field(c,'Название новой копии','Рабочая — восстановленная');field(c,'Новый пароль','••••••••••••',{secret:true});field(c,'Повтор пароля','••••••••••••',{secret:true});paragraph(c,'Старые допуски не переносятся. Другие устройства потребуется подключить заново. Исходная копия сохранится.');},{label:'Создать'});
  form('transfer-control','Передать управление',c=>{paragraph(c,'Рабочая · сейчас управляет Pixel 9');item(c,'MacBook Pro','В сети · разблокирован','monitor',{selected:true});paragraph(c,'Оба устройства должны быть подключены и разблокированы. MacBook Pro подтвердит передачу.');},{label:'Передать'});
  confirm('accept-control','Принять управление?','MacBook Pro передаёт управление БД «Рабочая» этому Pixel 9. После подтверждения этот телефон будет единственным управляющим устройством.','Принять');
  const invitation=phone('invite','Пригласить устройство',{page:FORMS,right:null});paragraph(invitation,'Рабочая · одноразовый код\nОсталось 04:32 из 05:00');
  const q=box(invitation.body,'QR / synthetic invitation',75,invitation.y,240,240,'#FFFFFF');
  const modules=DEMO_INVITATION.modules,unit=240/(modules.length+8),paths=[];
  modules.forEach((row,y)=>[...row].forEach((on,x)=>{if(on==='1'){const xx=(x+4)*unit,yy=(y+4)*unit;paths.push(`M${xx} ${yy}L${xx+unit} ${yy}L${xx+unit} ${yy+unit}L${xx} ${yy+unit}Z`);}}));
  const vector=figma.createVector();q.appendChild(vector);vector.vectorPaths=[{windingRule:'NONZERO',data:paths.join(' ')}];vector.fills=paint('#000000');vector.strokes=[];vector.x=4*unit;vector.y=4*unit;invitation.y+=260;
  field(invitation,'Код приглашения',DEMO_INVITATION.code.match(/.{1,28}/g).join('\n'),{edit:false,lines:4});fullAction(invitation,'Копировать код');paragraph(invitation,'Подключение требует подтверждения управляющим устройством.');finish(invitation);
  form('confirm-device','Подтвердить устройство',c=>{heading(c,'MacBook Pro');paragraph(c,'Запрашивает доступ к базе «Рабочая».');field(c,'Код проверки','742 618',{edit:false});paragraph(c,'Сверьте код на обоих устройствах.');},{label:'Разрешить',form:{label:'Разрешить',cancel:'Отклонить'}});
  form('conflict-custom','Своё значение',c=>{paragraph(c,'GitHub · конфликт пароля\nОригинальные варианты сохранятся в истории.');field(c,'Новый итог','••••••••••••',{secret:true});},{label:'Применить'});
  form('conflict-deletion','Удаление и правка',c=>{heading(c,'Старый сервер');paragraph(c,'Объект находится в корзине. Pixel 9 удалил его, MacBook Pro изменил пароль.');field(c,'Сохранённый пароль','••••••••••••',{edit:false,actions:[['eye','Показать вариант'],['copy','Копировать вариант']]});choice(c,'Восстановить с правками',false,{radio:true});choice(c,'Подтвердить удаление',false,{radio:true});},{label:'Применить',kind:'disabled'});
  form('conflict-placement','Конфликт размещения',c=>{heading(c,'Инфраструктура');choice(c,'Работа / Сервисы',false,{radio:true});choice(c,'Личное / Сервисы',false,{radio:true});fullAction(c,'Выбрать другую группу…');paragraph(c,'Недопустимое размещение в собственном поддереве исключено.');},{label:'Применить',kind:'disabled'});
  form('extract-pending','Извлечь правки',c=>{paragraph(c,'Источник: Mac mini · Старый сервер');field(c,'Название новой записи','Старый сервер — восстановлен');field(c,'Группа','Выберите группу…',{edit:false,select:true});paragraph(c,'Новая запись сохранит происхождение правок. Остальной источник останется доступным.');},{label:'Извлечь',kind:'disabled'});
  form('database-info','Название базы',c=>{field(c,'Название','Рабочая');field(c,'Описание','Рабочие сервисы и инфраструктура',{lines:3});});
  form('relay','Собственный relay',c=>{field(c,'Адрес','https://relay.example',{lines:2});paragraph(c,'Relay пересылает зашифрованный трафик. Если прямой связи нет, отключение relay оставляет устройства без обмена.');},{label:'Применить'});
  form('import','Импортировать KDBX',c=>{fullAction(c,'Выбрать KDBX…');paragraph(c,'Рабочая.kdbx · KDBX 4.1');field(c,'Пароль источника','••••••••••••',{secret:true});fullAction(c,'Выбрать ключевой файл…');paragraph(c,'Допустим пароль, ключевой файл или оба. KDBX 3.1 / 4.0 / 4.1.');},{label:'Продолжить'});
  form('import-review','Новая база Taypeer',c=>{paragraph(c,'Будет перенесено: 4 группы, 9 записей, 2 вложения и 12 версий.');field(c,'Название','Импортированная');field(c,'Мастер-пароль Taypeer','••••••••••••',{secret:true});field(c,'Повтор пароля','••••••••••••',{secret:true});fullAction(c,'Подробный отчёт…');paragraph(c,'Создаётся отдельная внутренняя БД. Выбранный KDBX не перезаписывается.');},{label:'Импортировать'});
  form('export','Экспортировать KDBX',c=>{paragraph(c,'Рабочая → KDBX 4.1');field(c,'Пароль KDBX','••••••••••••',{secret:true});field(c,'Повтор пароля','••••••••••••',{secret:true});paragraph(c,'Место сохранения выбирается системным диалогом после отчёта.');},{label:'Продолжить'});
  form('transfer-report','Отчёт экспорта',c=>{heading(c,'KDBX 4.1');paragraph(c,'Группы, поля, защищённые атрибуты, вложения, теги, история и корзина переносятся.','fg',16);notice(c,'Данные без эквивалента','CRDT-связи, доверенные устройства, источник значков и идентичность Lucide не переносятся.');paragraph(c,'Конфликтов нет.');},{label:'Создать KDBX'});
  form('import-report','Отчёт импорта',c=>{heading(c,'Рабочая.kdbx');paragraph(c,'Переносятся 4 группы, 9 записей, 2 вложения и 12 сохранённых версий. Защищённые атрибуты остаются защищёнными.','fg',16);notice(c,'Неподдерживаемые возможности','Демонстрационный источник: настройки Auto-Type и свойства плагина будут пропущены.');paragraph(c,'Аппаратные ключи и ключи плагинов не поддерживаются. Такой источник нельзя открыть для импорта.');},{label:'Продолжить'});
  form('migrate','Обновить формат',c=>{paragraph(c,'Рабочая · управляющее устройство');paragraph(c,'Старые приложения могут потерять возможность записи. История, конфликты и неприменённые правки сохранятся.','fg',16);paragraph(c,'Остальные устройства получат переход при следующем обмене. Автоматическая резервная копия не создаётся.');fullAction(c,'Сохранить копию .taypeer…');},{label:'Обновить'});

  function sheet(name,title,items,{note='',selected,destructive=[]}={}) {
    const c=phone(name,'Работа',{page:STATES,menu:true});search(c);item(c,'GitHub','demo@studio.example');finish(c);
    const dim=box(c.p,'Android / modal scrim',0,32,390,788,'#000000');dim.opacity=0.72;
    const noteHeight=note?Math.ceil(note.length*8/342)*21+12:0,h=76+items.length*52+noteHeight;
    const n=box(c.p,'Android / bottom sheet',0,820-h,390,h,'panel',16);
    box(n,'Sheet handle',175,8,40,4,'muted',2);copy(n,title,24,28,342,{size:18,weight:500,h:28});
    items.forEach((s,i)=>action(n,s,16,68+i*52,358,s===selected?'selected':destructive.includes(s)?'danger':'ghost'));
    if(note)copy(n,note,24,68+items.length*52,342,{size:14,color:'muted',h:noteHeight-4});return c.p;
  }
  sheet('database-switcher','Базы данных',['Рабочая · открыта','Личная · заблокирована','Открыть .taypeer…','Создать базу…','Получить с устройства…','Закрыть текущую базу'],{selected:'Рабочая · открыта'});
  sheet('entry-menu','GitHub',['Изменить…','Клонировать…','Переместить…','Удалить в корзину…'],{destructive:['Удалить в корзину…']});
  sheet('group-menu','Работа',['Создать подгруппу…','Изменить группу…','Клонировать группу…','Переместить группу…','Загрузить значки…','Удалить группу…'],{destructive:['Удалить группу…']});
  sheet('sort','Сортировка',['Название · А–Я','Название · Я–А','Логин · А–Я','Изменение · новые первыми'],{selected:'Название · А–Я'});
  sheet('search-scope','Область поиска',['Текущая БД','Все разблокированные БД'],{selected:'Текущая БД',note:'Заблокированные базы и защищённые значения исключены.'});
  sheet('attachment-menu','recovery-codes.txt',['Сохранить наружу…','Переименовать…','Заменить файлом…','Удалить вложение…'],{destructive:['Удалить вложение…']});
  sheet('attribute-menu','Recovery code',['Изменить атрибут…','Удалить атрибут'],{destructive:['Удалить атрибут']});
  sheet('theme','Тема',['Системная','Светлая','Тёмная'],{selected:'Системная'});
  sheet('expiry','Срок действия',['Бессрочно','Через 30 дней','Через 90 дней','Выбрать дату…'],{selected:'Бессрочно'});
  sheet('unsaved','Сохранить изменения?',['Сохранить','Не сохранять','Остаться'],{destructive:['Не сохранять'],note:'Переход продолжится после успешного сохранения.'});
  sheet('delete','Удалить GitHub?',['Удалить в корзину','Отмена'],{destructive:['Удалить в корзину'],note:'Запись можно восстановить из корзины.'});
  sheet('restore-history','Восстановить версию?',['Восстановить','Отмена'],{note:'Выбранная версия станет новой текущей. История сохранится.'});
  sheet('purge','Очистить корзину?',['Очистить окончательно','Отмена'],{destructive:['Очистить окончательно'],note:'Операция не стирает старые файлы и копии на других устройствах.'});
  sheet('clear-history','Очистить историю GitHub?',['Очистить историю','Отмена'],{destructive:['Очистить историю'],note:'Текущая запись и неприменённые источники сохранятся.'});
  sheet('delete-source','Удалить источник?',['Удалить источник','Отмена'],{destructive:['Удалить источник'],note:'Отложенные правки Mac mini больше не будут доступны для извлечения.'});
  sheet('duplicate-import','База уже импортирована',['Открыть внутреннюю копию','Отмена'],{note:'Выбранный файл относится к «Рабочая». Рабочая копия уже есть на устройстве.'});
  sheet('draft','Найден локальный черновик',['Продолжить правки','Удалить черновик','Позже'],{destructive:['Удалить черновик'],note:'Черновик зашифрован на этом устройстве и ещё не синхронизирован.'});
  sheet('device-menu','MacBook Pro',['Синхронизировать','Передать управление…','Отозвать устройство…'],{destructive:['Отозвать устройство…']});
  sheet('icon-picker','Значок',['Стандартный значок','Lucide Icons…','Загруженный favicon','Загрузить по URL…','Выбрать файл…']);
  form('lucide-picker','Значки Lucide',c=>{search(c,'Найти значок…');for(const [i,key] of ['key-round','folder','file','database','monitor','smartphone','lock-keyhole','shield-check','history','share-2','camera','globe'].filter(k=>mobileIcons[k]).entries()){const n=touch(c.body,'Выбрать значок '+key,16+i%5*68,c.y+Math.floor(i/5)*68,56,56,i===0?'selected':'ghost');glyph(n,key,18,18);}c.y+=220;},{label:'Выбрать'});
  form('color','Цвет записи',c=>{field(c,'HEX / RGBA','#E6E6E6FF');heading(c,'Палитра');for(const [i,color] of ['#E6E6E6','#D0B57D','#8BB694','#EE9292','#739AD4'].entries()){const n=touch(c.body,'Цвет '+color,16+i*68,c.y,56,56);box(n,'Swatch',8,8,40,40,color,4);}c.y+=80;fullAction(c,'По умолчанию');},{label:'Применить'});
  // State panels are component specimens, not additional application routes.
  function stateSheet(name,title,samples) {
    const p=board(STATES,'Android states / '+name,1260,1280);copy(p,title,24,24,1212,{size:26,weight:500,h:40});
    samples.forEach(([title,render],i)=>{const x=24+i%3*412,y=100+Math.floor(i/3)*570;copy(p,title,x,y,388,{size:16,weight:500,h:48});const n=box(p,'Android / specimen',x,y+56,388,480,'bg',6,true);const c={p:n,body:n,w:388,y:16};render(c);});return p;
  }
  stateSheet('session','Вход, запись и локальный черновик',[
    ['Неверный пароль',c=>{field(c,'Мастер-пароль','••••••••',{secret:true});paragraph(c,'Неверный пароль','danger');fullAction(c,'Повторить','primary');fullAction(c,'Ввести мастер-пароль');}],
    ['Защита изменилась',c=>{paragraph(c,'Получено новое состояние защиты. Введите актуальный мастер-пароль.','fg',16);field(c,'Мастер-пароль','',{secret:true});fullAction(c,'Разблокировать','primary');}],
    ['Не удалось сохранить',c=>{notice(c,'Недостаточно места','Черновик остаётся открытым. Подтверждённая запись не изменилась.');fullAction(c,'Повторить','primary');fullAction(c,'Остаться в редакторе');}],
    ['Ошибка черновика при блокировке',c=>{heading(c,'База заблокирована');paragraph(c,'Не удалось сохранить черновик. Его восстановление не гарантируется.','danger',16);fullAction(c,'Разблокировать…');}],
    ['Сохранение / ожидание',c=>{heading(c,'Сохраняем изменения…');paragraph(c,'Подтверждение временно недоступно.');fullAction(c,'Сохранить','disabled');}],
    ['Создание и локальная копия',c=>{heading(c,'База создана');paragraph(c,'Рабочая · внутренняя копия\nИсходный файл импорта не обновляется.');fullAction(c,'Открыть');fullAction(c,'Сохранить копию .taypeer…');}]
  ]);
  stateSheet('exchange','Обмен: от приглашения до применения',[
    ['Допуск получателя',c=>{heading(c,'Ожидаем подтверждения');paragraph(c,'Сверьте код на MacBook Pro.');field(c,'Код проверки','742 618',{edit:false});fullAction(c,'Отменить');}],
    ['Истекло / отказ / использовано',c=>{heading(c,'Приглашение истекло');paragraph(c,'Попросите новый код. При отказе или уже использованном коде подключение также не выполняется.');fullAction(c,'Ввести новый код…');}],
    ['Ожидание и сеть',c=>{heading(c,'Пир не в сети');paragraph(c,'Повторим при доступном соединении. Локальная работа доступна.');fullAction(c,'Повторить');paragraph(c,'Нет прямой связи · relay отключён');fullAction(c,'Настройки relay…');}],
    ['Приём на заблокированном устройстве',c=>{heading(c,'Изменения получены');paragraph(c,'Зашифрованные данные надёжно сохранены. Требуется вход для применения.');fullAction(c,'Разблокировать…');}],
    ['После входа',c=>{heading(c,'Изменения применены');paragraph(c,'Обновлённые записи доступны.');notice(c,'Есть 3 конфликта','Выберите итог для спорных полей.',{button:'Разобрать…'});}],
    ['Ошибка / обрыв',c=>{heading(c,'Получение не завершено');paragraph(c,'Ошибка записи. Получение не подтверждено; загрузка продолжится после повтора.','danger',16);fullAction(c,'Продолжить');paragraph(c,'Прерванная передача управления продолжается как та же операция.');}]
  ]);
  stateSheet('entry','Состояния полей, вложений и конфликтов',[
    ['Атрибуты: ошибка и защита',c=>{field(c,'Название','Recovery code');paragraph(c,'Такое имя уже есть','danger');choice(c,'Защищённый атрибут',true);paragraph(c,'При конфликте защиты варианты остаются скрытыми.');}],
    ['Вложение: лимит',c=>{item(c,'archive.zip','12 МиБ · лимит 10 МиБ','file');paragraph(c,'Файл не добавлен','danger',16);fullAction(c,'Выбрать другой файл…');fullAction(c,'Лимиты БД…');}],
    ['Вложение ещё не получено',c=>{item(c,'recovery-codes.txt','Содержимое ожидает получения','file');fullAction(c,'Получить');paragraph(c,'Сохранение наружу недоступно до получения полного содержимого.');}],
    ['Генератор: пустой алфавит',c=>{heading(c,'Нет доступных символов');paragraph(c,'Включите набор или измените исключения.');fullAction(c,'Сгенерировать','disabled');fullAction(c,'Использовать','disabled');}],
    ['Срок и длинное значение',c=>{choice(c,'Срок действия включён',true);field(c,'Дата','15 октября 2026',{select:true,edit:false});fullAction(c,'Показать значение целиком…');paragraph(c,'Длинные пути переносятся. Копирование сохраняет исходное значение.');}],
    ['Загрузка иконок',c=>{heading(c,'Обработано 4 из 6 записей');paragraph(c,'3 значка загружены\n1 адрес недоступен');fullAction(c,'Повторить ошибки');paragraph(c,'Успешные значки сохранены. Открытие записи само не загружает иконки.');}]
  ]);
  stateSheet('compatibility','Права, совместимость и перенос файлов',[
    ['Обычное доверенное устройство',c=>{heading(c,'Управляет MacBook Pro');paragraph(c,'Приглашение, отзыв, пароль, защита, общие лимиты и миграция доступны на управляющем устройстве.');fullAction(c,'Устройства…');}],
    ['Файл без сетевого допуска',c=>{heading(c,'Локальное чтение доступно');paragraph(c,'Файл и пароль не подключают устройство к обмену.');fullAction(c,'Получить приглашение…');fullAction(c,'Восстановить управление…');}],
    ['Чтение без записи',c=>{heading(c,'Только чтение');paragraph(c,'Запись этой схемы не поддерживается.');fullAction(c,'Редактировать','disabled');paragraph(c,'При неподдерживаемом чтении требуется обновление приложения.');}],
    ['Неподдерживаемый / повреждённый файл',c=>{heading(c,'Не удалось открыть файл');paragraph(c,'Версия не поддерживается. Исходный файл не изменён.');fullAction(c,'Выбрать другой файл…');paragraph(c,'Повреждение показывается отдельной причиной; перезаписи нет.');}],
    ['Экспорт блокируют конфликты',c=>{heading(c,'Сначала разрешите конфликты');paragraph(c,'KDBX не может сохранить нерешённые варианты.');fullAction(c,'Разобрать…');fullAction(c,'Создать KDBX','disabled');}],
    ['Отложенные источники / расхождение',c=>{heading(c,'Требуется управляющее устройство');paragraph(c,'Правки старой схемы сохранены и ждут разбора.');fullAction(c,'Отложенные правки…');paragraph(c,'При расхождении управления обмен прекращается; локальное чтение доступно.');fullAction(c,'Восстановить управление…');}]
  ]);
  stateSheet('system','Android · системные поверхности и возврат',[
    ['Системный выбор файла · схема',c=>{heading(c,'Открыть файл');item(c,'Рабочая.taypeer','Загрузки · 2,4 МиБ','file');item(c,'Рабочая.kdbx','Документы · 3,1 МиБ','file');fullAction(c,'Выбрать');paragraph(c,'Отмена возвращает в вызвавшую форму.');}],
    ['Камера и разрешение · схема',c=>{const n=box(c.body,'System / camera illustration',16,c.y,356,160,'panel',4,true);box(n,'Scan frame',110,18,124,124,'',4,true);c.y+=176;paragraph(c,'Наведите камеру на QR');fullAction(c,'Ввести код…');paragraph(c,'При отказе в разрешении камера закрыта, ввод кода доступен.');}],
    ['Биометрия · схема',c=>{glyph(c.body,'fingerprint',174,24);c.y=70;heading(c,'Подтвердите личность');paragraph(c,'Биометрия для базы «Рабочая».');fullAction(c,'Использовать мастер-пароль');fullAction(c,'Отмена');}],
    ['Активный обмен · уведомление',c=>{heading(c,'Taypeer · обмен');paragraph(c,'Получение зашифрованных изменений…');fullAction(c,'Остановить');paragraph(c,'Без названий записей, секретов и кодов приглашений.');}],
    ['Фон / недавние приложения',c=>{glyph(c.body,'lock-keyhole',174,32);c.y=80;heading(c,'Taypeer');paragraph(c,'База заблокирована.');fullAction(c,'Вернуться в приложение');paragraph(c,'После возврата — вход, затем предложение локального черновика.');}],
    ['Явная внешняя копия',c=>{heading(c,'Сохранить копию');field(c,'Имя файла','Рабочая.taypeer',{edit:false});fullAction(c,'Выбрать место…');paragraph(c,'Успех только после завершения записи. Отмена не меняет рабочую копию.');}]
  ]);
  // More control specimens on the shared mobile library sheet.
  const kc={p:kit,body:kit,w:390,y:570};field(kc,'Фокус','Введённое значение',{focus:true});choice(kc,'Независимое действие',true);fullAction(kc,'Подтвердить','primary');
  copy(kit,'48 dp проверяется у всего действия.\nОбласть ввода заканчивается перед отдельными\nкнопками показа, копирования и генерации.\n\nПять полных названий вкладок в одной полосе.\nСкрытая часть доступна горизонтальной прокруткой.',440,586,760,{size:16});
  tabsFor(kit,440,802,740,'Дополнительно');

  // Control sizes are laid out again, not scaled screenshots.
  entriesScreen('small-360-640',{page:QA,w:360,h:640});entriesScreen('large-430-932',{page:QA,w:430,h:932,long:true});
  entriesScreen('landscape-844-390',{page:QA,w:844,h:390});drawer('small-drawer',180,{page:QA,w:360,h:640});
  const keyboard=phone('keyboard','GitHub',{page:QA,right:null,keyboard:true,form:{label:'Сохранить'},tab:'Обзор'});field(keyboard,'Логин','demo@studio.example',{entry:true});field(keyboard,'Пароль','••••••••••••',{entry:true,actions:[['eye','Показать пароль'],['dice-5','Генератор пароля']]});field(keyboard,'URL','https://github.com',{entry:true,focus:true,actions:[['download','Загрузить favicon…']]});finish(keyboard);
  const smallKeyboard=phone('small-keyboard','Создать группу',{page:QA,w:360,h:640,right:null,keyboard:true,form:{label:'Создать'}});field(smallKeyboard,'Название','Инфраструктура',{focus:true});finish(smallKeyboard);
  const horizontalKeyboard=phone('landscape-keyboard','GitHub',{page:QA,w:844,h:390,right:null,keyboard:true,form:{label:'Сохранить'}});field(horizontalKeyboard,'URL','https://github.com',{entry:true,focus:true});finish(horizontalKeyboard);
  function largeHeader(c) {
    const bar=c.p.children.find(n=>n.name==='Android / app bar');
    const title=bar.children.find(n=>n.type==='TEXT');title.fontSize=32;title.resize(title.width,48);title.y=4;
  }
  const largeText=phone('text-200','Работа',{page:QA,menu:true});largeHeader(largeText);
  const ls=inputFrame(largeText.body,'Поиск 200%',16,16,358,72);const lt=touch(ls,'Поиск',0,0,358,72);glyph(lt,'search',14,26);copy(lt,'Поиск…',48,12,294,{size:32,h:48});
  const lr=touch(largeText.body,'Инфраструктура и рабочие сервисы',16,112,358,288);
  copy(lr,'Инфраструктура\nи рабочие\nсервисы',16,12,326,{size:32,weight:500,h:144});
  copy(lr,'demo@studio.\nexample',16,180,326,{size:28,color:'muted',h:84});largeText.y=424;finish(largeText);
  function largeForm(name,keyboard=false) {
    const c=phone(name,'GitHub',{page:QA,right:null,keyboard});largeHeader(c);
    const bottom=keyboard?540:820,actions=box(c.p,'Android / form actions',0,bottom-160,390,160,'panel');
    c.viewport.resize(390,actions.y-c.viewport.y);
    for(const [i,s] of ['Сохранить','Отмена'].entries()){const n=touch(actions,s,16,8+i*76,358,68,i===0?'primary':'ghost');copy(n,s,16,10,326,{size:32,color:i===0?'onPrimary':'fg',h:48});}
    copy(c.body,'Заметки',16,16,358,{size:28,color:'muted',h:42});
    const input=inputFrame(c.body,'Заметки 200%',16,66,358,164,'row','focus');
    const target=touch(input,'Ввести заметки',0,0,358,164);copy(target,'Рабочий аккаунт\nдля тестового\nсервиса.',0,8,350,{size:32,h:144});c.y=246;return finish(c);
  }
  largeForm('text-200-form');largeForm('text-200-keyboard',true);
  const bigDrawer=phone('text-200-drawer','Работа',{page:QA,menu:true});bigDrawer.viewport.remove();
  const bd=box(bigDrawer.p,'Android / drawer viewport',0,32,342,788,'panel');bd.clipsContent=true;
  const bc=box(bd,'Android / scroll content',0,0,342,1320,'panel');bigDrawer.viewport=bd;bigDrawer.body=bc;bigDrawer.w=342;
  const db=touch(bc,'Выбрать базу',8,8,326,124);copy(db,'Рабочая',16,8,294,{size:32,h:48});copy(db,'Внутренняя\nкопия',16,60,294,{size:28,color:'muted',h:84});db.resize(326,152);
  let gy=176;
  for(const [name,chosen] of [['Работа',true],['Сервисы',false],['Инфраструктура',false]]){const n=box(bc,'Android / group row',8,gy,326,104,chosen?'selected':'');iconAction(n,'chevron-down','Раскрыть '+name,0,28);const l=touch(n,'Открыть группу '+name,48,0,278,104);copy(l,name==='Инфраструктура'?'Инфраструк-\nтура':name,8,8,262,{size:32,h:96});gy+=104;}
  for(const s of ['Создать группу…','Устройства и обмен','Корзина','Конфликты','Настройки','Заблокировать']){const n=touch(bc,s,8,gy,326,112);copy(n,s,16,8,294,{size:32,h:96});gy+=112;}
  bigDrawer.y=gy;finish(bigDrawer);
  const long=phone('long-values','Инфраструктура и сервисы',{page:QA,right:null});field(long,'Путь группы','Рабочая / Инфраструктура /\nПродакшен / Внешние сервисы',{edit:false,lines:3});field(long,'URL','https://service.example/teams/\ninfrastructure/production/\ncredentials?workspace=studio',{edit:false,lines:4});fullAction(long,'Показать целиком…');finish(long);
  const light=LIGHT_PALETTE;
  const collection=figma.createVariableCollection('Taypeer · Android / light');const lv=Object.fromEntries(Object.entries(light).map(([k,v])=>[k,figma.createVariable(k,'COLOR',collection.id,{...rgb(v),a:1})]));
  function recolor(n) {for(const prop of ['fills','strokes']){if(!Array.isArray(n[prop]))continue;const roles=n[prop].map(v=>v.type==='SOLID'?Object.entries(P).find(([,h])=>['r','g','b'].every(k=>Math.abs(rgb(h)[k]-v.color[k])<0.001))?.[0]:null);n[prop]=n[prop].map((v,i)=>roles[i]?{...v,color:rgb(light[roles[i]])}:v);roles.forEach((r,i)=>{if(r)figma.bindVariable(n.id,`${prop}/${i}/color`,lv[r].id);});}if(n.children)n.children.forEach(recolor);}
  for(const source of ['entries','drawer','edit','unlock','settings','conflict']){const original=made.find(n=>n.name==='Android / '+source);const n=original.clone();pages[QA].appendChild(n);n.name='Android QA / light-'+source;recolor(n);boards.push({name:n.name,id:n.id,page:QA});}

  const flow=board(FLOW,'Android flow / v1',1320,1860);copy(flow,'Android · карта сценариев v1',24,24,1272,{size:28,weight:500,h:48});copy(flow,'Телефоны · русский · тёмная тема + контрольные светлые экраны',24,84,1272,{color:'muted'});
  const routes=[
    ['Начало','welcome → create / системный файл\n→ duplicate-import / unlock → empty\n→ group → new-entry'],
    ['Навигация','entries ↔ drawer ↔ database-switcher\nСтрелка раскрывает; название открывает.\nFAB → new-entry · меню → sort'],
    ['Поиск','search-current ↔ search-scope\n→ search-all / search-empty → entry\nНазад возвращает запрос и область.'],
    ['Запись','entry ↔ edit / new-entry\nadvanced ↔ advanced-edit\nappearance ↔ appearance-edit'],
    ['Поля и файлы','attribute · attachment-menu · expiry\nicon-picker → color / image-url\ngenerator ↔ passphrase → использовать'],
    ['История и корзина','history → history-version → compare\n→ restore-history → новая версия\ntrash → destination / purge'],
    ['Подключение','receive → камера / код → confirm-device\n→ receiving → unlock → применение\ninvite · 5 минут · один получатель'],
    ['Управление','devices → device-menu → revoke\ntransfer-control → accept-control\nrecover-control → новый набор'],
    ['Конфликты','entries-conflicts → conflicts\n→ conflict / deletion / placement\nЯвный выбор → надёжная запись'],
    ['Отложенные правки','pending → pending-source\n→ extract-pending → destination\nили delete-source → подтверждение'],
    ['Файлы и настройки','settings ↔ settings-database → формы\nfiles → import → review / export → report\n.taypeer → системное сохранение'],
    ['Совместимость','Чтение / только чтение / обновить\nУправляющее → migrate → запись\nСтарая схема → pending → разбор']
  ];
  routes.forEach(([title,steps],i)=>{const n=box(flow,'Android flow / '+title,24+i%3*432,144+Math.floor(i/3)*304,408,280,'panel',6,true);copy(n,title,16,16,376,{size:20,weight:500});copy(n,steps,16,72,376,{size:16,h:168});});
  copy(flow,'Правила переходов',24,1400,1272,{size:22,weight:500});
  copy(flow,'Назад: клавиатура → верхний слой → предыдущий маршрут. Изменённая форма: Сохранить / Не сохранять / Остаться.\nСохранение продолжает переход только после валидации и надёжной записи. Черновик общий для трёх редактируемых вкладок.\nФон блокирует БД сразу; зашифрованный черновик предлагается после входа. Уведомление обмена не раскрывает содержимое.\nМеню открывается кнопкой и свайпом вправо внутри списка, вне системного края. В записи и редакторе свайп меню выключен.\nКраевой Back, Home, клавиатура, TalkBack и фон проверяются на реальном Android отдельно от этих статических схем.',24,1456,1272,{size:16,h:216});
  copy(flow,'Контрольные размеры: 390 × 844 · 360 × 640 · 430 × 932 · 844 × 390.\n48 dp без пересечения действий. При 200% строки растут, формы прокручиваются, команды переносятся на две строки.\nМатрица: wireframes/COVERAGE.md · приёмка устройства: wireframes/ANDROID-QA.md',24,1710,1272,{size:16,color:'muted',h:120});
  // Arrange only Android pages; preserve every desktop artboard and its position.
  for(const name of [MAIN,FORMS,STATES,QA,FLOW]){const nodes=pages[name].children;const cols=name===STATES?3:name===FLOW?1:4,cw=Math.max(...nodes.map(n=>n.width))+64;let y=0;for(let i=0;i<nodes.length;i+=cols){const row=nodes.slice(i,i+cols);row.forEach((n,j)=>{n.x=j*cw;n.y=y;});y+=Math.max(...row.map(n=>n.height))+80;}counts[name]=nodes.length;}
  const overview=pages['00 · Начать здесь'].children[0];
  const line=overview.findAll(n=>n.type==='TEXT'&&n.characters==='Исходные мобильные макеты')[0];
  if(line)line.characters='Телефоны: записи, дерево в меню, формы, состояния и контрольные размеры · страницы 02, 09–12';
}
