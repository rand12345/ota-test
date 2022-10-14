# ota-test

Update url http://\<ESP-IP\>/ota

Update using cURL (WIP)

```curl -F file=@app.bin http://\<ESP-IP\>/otaupload```

Use cargo to create .bin files 

```cargo espflash save-image ota.bin```
