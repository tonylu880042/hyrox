// node --test apps/hub-server/tests/ui/training-actions.test.cjs
const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const vm=require('node:vm');
function page(){
 const html=fs.readFileSync(require('node:path').resolve(__dirname,'../../static/training.html'),'utf8');
 const script=html.match(/<script>\s*([\s\S]*?)<\/script>/)[1];
 const c=vm.createContext({document:{getElementById(){}},I18N:{t:(key,...args)=>key+args.join('|')}});
 vm.runInContext(script.split('let PAGE_SIZE')[0],c);
 return code=>vm.runInContext(code,c);
}
const setup=`const course=[{name:'PUSH',plan:'25 M'},{name:'PULL',plan:'30 M'}]; const a={name:'Alice',bib:1,status:'ACTIVE',station_state:'INSIDE',station:'PUSH',station_key:'push',station_index:0,completed:0,plan:'25 M',next_station:'PULL',next_plan:'30 M',elapsed_ms:1000,leg_ms:1000}; const s={mode:'TRAINING',status:'RUNNING',course,athletes:[a]};`;
test('training cards preview the next action while exercising, competition stays unchanged',()=>{
 const p=page();p(setup);
 assert.match(p('card(a,course,true)'),/PULL/);
 assert.match(p('card(a,course,true)'),/30 unit.METER/);
 assert.doesNotMatch(p('card(a,course,false)'),/live.nextAction/);
});
test('the shared action is shown only when every entrant is at the same step and state',()=>{
 const p=page();p(setup);
 assert.equal(p('trainingSummary(s).kind'),'shared');
 p('s.athletes.push({...a,bib:2,station_index:1,completed:1})');
 assert.equal(p('trainingSummary(s).kind'),'mixed');
 p('s.athletes[1]={...a,bib:2,status:"READY",station_state:"OUTSIDE"}');
 assert.equal(p('trainingSummary(s).kind'),'mixed');
});
test('ready previews the first step; completed and empty courses never suggest further work',()=>{
 const p=page();p(setup);
 p('a.status="READY"; a.station_state="OUTSIDE"');
 assert.equal(p('trainingSummary(s).kind'),'ready');
 p('s.status="COMPLETED"');
 assert.equal(p('trainingSummary(s).kind'),'complete');
 p('s.course=[];s.status="DRAFT"');
 assert.equal(p('trainingSummary(s).kind'),'empty');
});
test('the final station says it is last and finished athletes do not preview another station',()=>{
 const p=page();p(setup);
 p('a.completed=1;a.station_index=1;a.next_station=null;a.next_plan=null');
 assert.match(p('card(a,course,true)'),/live.lastAction/);
 p('a.status="FINISHED"');
 assert.doesNotMatch(p('card(a,course,true)'),/live.nextAction|live.lastAction/);
});
test('transition previews its destination without claiming it is already the current action',()=>{
 const p=page();p(setup);
 p('a.station_state="OUTSIDE";a.completed=1');
 assert.equal(p('trainingSummary(s).kind'),'transition');
 assert.equal(p('trainingSummary(s).next'),'PULL');
});
test('action labels from the server are escaped in new UI',()=>{
 const p=page();p(setup);
 p('a.next_station="<img src=x onerror=alert(1)>"');
 assert.match(p('card(a,course,true)'),/&lt;img/);
 assert.doesNotMatch(p('card(a,course,true)'),/<img src=x/);
});
