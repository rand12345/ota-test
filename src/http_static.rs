pub const SERVER: &str = r#"<script src='https://ajax.googleapis.com/ajax/libs/jquery/3.2.1/jquery.min.js'></script>
<form method='POST' action='/otaupload' enctype='multipart/form-data' id='upload_form'>
  <input type='file' name='update'>
       <input type='submit' value='Update'>
   </form>
<div id='prg'>progress: 0%</div>
<script>
 $('form').submit(function(e){
 e.preventDefault();
 var form = $('#upload_form')[0];
 var data = new FormData(form);
  $.ajax({
 url: '/otaupload',
 type: 'POST',
 data: data,
 contentType: false,
 processData:false,
 xhr: function() {
 var xhr = new window.XMLHttpRequest();
 xhr.upload.addEventListener('progress', function(evt) {
 if (evt.lengthComputable) {
 var per = evt.loaded / evt.total;
 $('#prg').html('progress: ' + Math.round(per*100) + '%');
 }
 }, false);
 return xhr;
 },
 success:function(d, s) {
 console.log('success!')
},
error: function (a, b, c) {
}
});
});
</script>"#;

pub const _SERVER_OLD: &str = r#"
              <form method="POST" action="/otaupload" enctype="text/plain"><input type="file" name="data"/><input type="submit" name="upload" value="Upload" title="Upload File"></form>
              <p>After clicking upload it will take some time for the file to firstly upload, there is no indicator that the upload began.  Please be patient.</p>
              <p>If a file does not appear, it will be because the file was too big, or had unusual characters in the file name (like spaces).</p>
              <p>You can see the progress of the upload by watching the serial output.</p>"#;
