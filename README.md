# ota-test

Update url http://\<ESP-IP\>/ota

Update using cURL (WIP)

```curl -F file=@app.bin http://<ESP-IP>/ota```

Use cargo to create .bin files 

```cargo espflash save-image ota.bin```

or one line flash and reset

```cargo espflash save-image ota.bin && curl -F file=@ota.bin http://<ESP-IP>/ota && curl http://<ESP-IP>/restart```
