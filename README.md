# ota-test

Update url http://\<ESP-IP\>/ota

Update using cURL (WIP)

```curl -F file=@app.bin http://\<ESP-IP\>/ota```

Use cargo to create .bin files 

```cargo espflash save-image ota.bin```

or one line flash and reset

```cargo espflash save-image ota.bin && curl -F file=@ota.bin http://10.0.1.164/ota && curl http://10.0.1.164/restart```
