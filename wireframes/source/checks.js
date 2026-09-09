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
return {overflow,narrow_text:tiny,design_contracts:{errors,measurements}};
