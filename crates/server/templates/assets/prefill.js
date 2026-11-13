{const h=new URLSearchParams(location.hash.slice(1));const c=h.get('code');const el=document.getElementById('code');if(c&&el){el.value=c;history.replaceState(null,'',location.pathname)}}
