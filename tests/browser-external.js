localStorage.setItem('theme','material');
document.cookie='sid=fixture';
document.getElementById('stored').textContent=localStorage.getItem('theme');
document.getElementById('cookies').textContent=document.cookie;
fetchText('/browser-data.txt','status');
