// Visual scene signature: omit volatile IDs, selection, viewport and page focus.
// The file hash additionally protects bytes of a saved document during publish.
const properties = ['type','name','x','y','width','height','rotation','visible','opacity',
 'fills','strokes','strokeWeight','strokeAlign','strokeCap','strokeJoin','effects',
 'cornerRadius','topLeftRadius','topRightRadius','bottomLeftRadius','bottomRightRadius',
 'clipsContent','layoutMode','primaryAxisSizingMode','counterAxisSizingMode',
 'primaryAxisAlignItems','counterAxisAlignItems','layoutAlign','layoutGrow',
 'paddingTop','paddingRight','paddingBottom','paddingLeft','itemSpacing',
 'characters','fontName','fontSize','textAutoResize','textAlignHorizontal',
 'textAlignVertical','lineHeight','letterSpacing','textDecoration','textCase','vectorPaths'];
function clean(value){
 if(typeof value==='number')return Number.isFinite(value)?Math.round(value*1e6)/1e6:null;
 if(Array.isArray(value))return value.map(clean);
 if(value&&typeof value==='object')return Object.fromEntries(Object.keys(value).sort().filter(k=>k!=='boundVariables').map(k=>[k,clean(value[k])]));
 return value;
}
function snapshot(node){
 const data={};
 for(const key of properties){const value=node[key];if(value!==undefined&&typeof value!=='function')data[key]=clean(value);}
 if(node.children)data.children=node.children.map(snapshot);
 return data;
}
return figma.root.children.map(snapshot);
