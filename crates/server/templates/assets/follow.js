setInterval(function(){fetch(location.href,{cache:'no-store'}).then(function(r){return r.text()}).then(function(t){if(t.indexOf('id="consent"')<0)location.reload()}).catch(function(){})},3000)
