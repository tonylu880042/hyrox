// Run with Playwright available on NODE_PATH. Uses isolated HTTP fixtures, no product DB.
const {chromium}=require('playwright');
const http=require('node:http');const fs=require('node:fs');const path=require('node:path');const assert=require('node:assert/strict');
const root=path.resolve(__dirname,'../../static');
const out=path.resolve(__dirname,'../../../..','design/demo/out/training-actions');fs.mkdirSync(out,{recursive:true});
const layouts=[{size:6,columns:3,rows:2},{size:12,columns:4,rows:3},{size:20,columns:5,rows:4},{size:30,columns:6,rows:5}];
const server=http.createServer((req,res)=>{
 const url=new URL(req.url,'http://localhost');
 if(url.pathname==='/api/settings'){res.setHeader('Content-Type','application/json');res.end(JSON.stringify({live_page_size:12,page_layouts:layouts}));return;}
 if(url.pathname==='/api/exercises'){res.setHeader('Content-Type','application/json');res.end(JSON.stringify({exercises:[{code:'SLED_PUSH',station_key:'SLED PUSH',display_name:'推雪橇'},{code:'SLED_PULL',station_key:'SLED PULL',display_name:'拉雪橇'}]}));return;}
 if(url.pathname==='/api/logo'){res.writeHead(404);res.end();return;}
 const file=path.join(root,url.pathname==='/live'?'training.html':url.pathname);
 if(!file.startsWith(root+path.sep)||!fs.existsSync(file)){res.writeHead(404);res.end();return;}
 const type={'.html':'text/html','.js':'text/javascript','.css':'text/css','.woff2':'font/woff2','.svg':'image/svg+xml'}[path.extname(file)]||'application/octet-stream';res.setHeader('Content-Type',type);fs.createReadStream(file).pipe(res);
});
const athlete={name:'陳怡君',bib:1,status:'ACTIVE',station_state:'INSIDE',station:'SLED PUSH',station_key:'sled_push',station_index:0,completed:0,plan:'25 M',next_station:'SLED PULL',next_plan:'30 M',elapsed_ms:25000,leg_ms:25000};
const base={session_name:'團課動作預覽・測試資料',mode:'TRAINING',status:'RUNNING',readers_online:6,class_elapsed_ms:120000,in_class:12,finished:0,exceptions:0,last_event_age_ms:0,course:[{name:'SLED PUSH',key:'sled_push',plan:'25 M'},{name:'SLED PULL',key:'sled_pull',plan:'30 M'}],athletes:[]};
(async()=>{
 await new Promise(r=>server.listen(0,'127.0.0.1',r));
 const browser=await chromium.launch({headless:true,executablePath:process.env.CHROME_PATH||'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'});
 try{
 const page=await browser.newPage({viewport:{width:1920,height:1080}});const errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.addInitScript(()=>{window.WebSocket=class{close(){}};});
 await page.goto(`http://127.0.0.1:${server.address().port}/live?lang=zh-Hant`);await page.evaluate(()=>document.fonts.ready);
 const results=[];
 for(const l of layouts){
  const s={...base,in_class:l.size,athletes:Array.from({length:l.size},(_,i)=>({...athlete,name:['陳怡君','林建宏','王淑芬','黃培綺'][i%4],bib:i+1}))};
  await page.evaluate(({s,l})=>{PAGE_SIZE=l.size;LAYOUT=l;document.getElementById('grid').style.gridTemplateColumns=`repeat(${l.columns},minmax(0,1fr))`;document.getElementById('grid').style.gridTemplateRows=`repeat(${l.rows},minmax(0,1fr))`;render(s);},{s,l});
  await page.screenshot({path:path.join(out,`shared-${l.size}.png`)});
  const clipping=await page.locator('.athlete-card').evaluateAll(cards=>cards.flatMap((c,i)=>{const body=c.getBoundingClientRect();const blocks=[...c.children].filter(x=>x.tagName==='DIV');const top=blocks[0].getBoundingClientRect();const bottom=blocks[1].getBoundingClientRect();return top.bottom>bottom.top+1||bottom.bottom>body.bottom+1?[{i,overlap:top.bottom-bottom.top,overflow:bottom.bottom-body.bottom}]:[];}));
  results.push({size:l.size,clipping});
 }
 const apply=async s=>page.evaluate(s=>render(s),s);
 await page.evaluate(()=>{PAGE_SIZE=12;LAYOUT={columns:4,rows:3};document.getElementById('grid').style.gridTemplateColumns='repeat(4,minmax(0,1fr))';document.getElementById('grid').style.gridTemplateRows='repeat(3,minmax(0,1fr))';});
 await apply({...base,athletes:[athlete,{...athlete,bib:2,completed:1,station:'SLED PULL',station_index:1,plan:'30 M',next_station:null,next_plan:null}]});
 await page.getByText('分站進行',{exact:true}).waitFor();await page.screenshot({path:path.join(out,'mixed.png')});
 await apply({...base,status:'PAUSED',athletes:[athlete]});assert.match(await page.locator('#training-overview').innerText(),/課程已暫停/);
 await apply({...base,status:'COMPLETED',athletes:[{...athlete,status:'FINISHED'}]});assert.match(await page.locator('#training-overview').innerText(),/課程已結束/);
 await apply({...base,athletes:[{...athlete,station_state:'OUTSIDE',completed:1}]});assert.match(await page.locator('#training-overview').innerText(),/轉換/);
 await apply({...base,mode:'COMPETITION',athletes:[athlete]});assert.equal(await page.locator('#training-overview').isVisible(),false);assert.equal(await page.locator('.next-action').count(),0);
 await apply({...base,athletes:[athlete]});await page.keyboard.press('l');assert.equal(await page.locator('#training-overview').isVisible(),false);await page.keyboard.press('l');assert.equal(await page.locator('#training-overview').isVisible(),true);
 await page.setViewportSize({width:1280,height:800});await page.screenshot({path:path.join(out,'laptop.png')});
 fs.writeFileSync(path.join(out,'browser-validation.json'),JSON.stringify({results,errors},null,2));console.log(JSON.stringify({results,errors}));assert.equal(errors.length,0);assert.ok(results.every(r=>!r.clipping.length),'cards must not clip or overlap');
 }finally{await browser.close();server.close();}
})().catch(e=>{console.error(e);server.close();process.exitCode=1;});
