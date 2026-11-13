(function(){var f=document.getElementById('consent');var at={view:0,ask:1,full:2};var $=function(id){return document.getElementById(id)};
function show(){var i=at[$('access').value];document.querySelectorAll('.perm').forEach(function(p){p.hidden=Number(p.dataset.level)>i});$('summary').innerHTML=f.getAttribute('data-s'+i);$('asks').textContent=f.getAttribute('data-a'+i)}
function said(id){var e=$(id);return e.options[e.selectedIndex].text}
function checked(cls){return Array.prototype.filter.call(document.querySelectorAll('.'+cls),function(c){return c.checked})}
var tried=false;
function sync(){var envs=document.querySelectorAll('.env');var on=checked('env');var all=on.length===envs.length;var ids=on.map(function(c){return c.value});
$('no-env').hidden=on.length>0||envs.length===0;$('env_ids').value=all?'':ids.join(',');
var names=on.map(function(c){return c.dataset.name});var where=all?'':' in '+names.join(', ');
var q=$('find').value.trim().toLowerCase();var visible=0,shown=0,inEnvs=[];document.querySelectorAll('#projects .prow').forEach(function(l){var keep=all||ids.indexOf(l.dataset.env)>=0;var box=l.querySelector('input');if(!keep)box.checked=false;else{visible++;inEnvs.push(box.value)}var match=keep&&(!q||l.dataset.find.indexOf(q)>=0);l.hidden=!match;if(match)shown++});
$('every').textContent='Every project'+where+' ('+visible+')';$('scope-hint').textContent=all?'From every environment.':'Only projects in '+names.join(', ')+', from step 1.';
var some=document.querySelector('input[name=scope]:checked').value==='some';$('pick').hidden=!some;$('projects').hidden=visible===0||shown===0;$('none-here').hidden=visible>0;$('no-match').hidden=visible===0||shown>0;
var picked=checked('proj');$('project_ids').value=some?picked.map(function(c){return c.value}).join(','):'';$('count').textContent=picked.length+' of '+visible+' picked';if(picked.length>0)tried=false;$('no-project').hidden=!(tried&&some&&picked.length===0);
var using=some?picked.map(function(c){return c.value}):inEnvs;var any=0;document.querySelectorAll('.use').forEach(function(r){var n=r.dataset.projects?r.dataset.projects.split(' ').filter(function(p){return using.indexOf(p)>=0}).length:0;r.hidden=n===0;if(n>0)any++;r.querySelector('.use-n').textContent=n+(n===1?' project':' projects')});if($('uses-none')){$('uses-none').hidden=any>0;$('uses-none').textContent=some&&picked.length===0?'Pick projects to see the accounts it will use.':'None of these projects runs on a connected account yet.'}
var go=document.querySelector('.go');go.disabled=on.length===0&&envs.length>0;
var projects=some?(picked.length===1?picked[0].dataset.name:picked.length+' projects'):(all?'Every project':'Every project'+where);
$('chosen').textContent=[said('access'),all?'Every environment':names.join(', '),projects,said('ends')].join(' · ')}
$('access').addEventListener('change',function(){show();sync()});$('choices').addEventListener('change',sync);$('find').addEventListener('input',sync);$('find').addEventListener('keydown',function(e){if(e.key==='Enter')e.preventDefault()});
$('edit').addEventListener('click',function(){var c=$('choices');c.hidden=!c.hidden;this.setAttribute('aria-expanded',String(!c.hidden))});
f.addEventListener('submit',function(e){var b=e.submitter;if(b&&b.value==='allow'&&document.querySelector('input[name=scope]:checked').value==='some'&&checked('proj').length===0){e.preventDefault();tried=true;var c=$('choices');c.hidden=false;$('edit').setAttribute('aria-expanded','true');sync();$('no-project').scrollIntoView({block:'center'});return}if(b&&b.value==='allow'){b.textContent='Confirm it’s you…';b.disabled=true;var h=document.createElement('input');h.type='hidden';h.name='decision';h.value='allow';f.appendChild(h)}});sync()})();
