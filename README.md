# ota-test

Update url http://\<ESP-IP\>/ota

Use esptool.py to create .bin files from ELF

```esptool.py --chip ESP32-C3 elf2image --output my-new-app.bin target/riscv32imc-esp-espidf/debug/ota-test```
