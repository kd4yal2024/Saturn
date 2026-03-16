Laurence Barker 27/6/2021:
build instructions for the XDMA driver

Preferred rebuild/update path:

sudo bash /home/pi/github/Saturn/scripts/fix-xdma.sh

This script:

- installs matching kernel headers when needed
- stops `p2app.service`, rebuilds and reinstalls `xdma.ko`, reloads the module, and restarts `p2app.service`
- when a newer kernel of the same Raspberry Pi flavor is already installed, also pre-stages XDMA for that kernel before reboot

Manual build path:

1. get the kernel headers so the kernel module can compile:
(note if this fails you will need to use an older OS release, or rebuild the kernel 
by following the instructions at https://www.raspberrypi.org/documentation/linux/kernel/building.md)


sudo apt install "linux-headers-$(uname -r)"


2. build the kernel module:

cd ~/github/Saturn/linuxdriver/xdma
make
sudo make install

If you need to build for a newer installed kernel before reboot, override the kernel build directory:

cd ~/github/Saturn/linuxdriver/xdma
make KDIR=/lib/modules/<kernel-version>/build
sudo make KDIR=/lib/modules/<kernel-version>/build install



3. copy the module "rules" files to /etc (among other things, this causes the access permissions to be changed when the module loads and the /dev/xdma devices are added)


sudo cp ../etc/udev/rules.d/* /etc/udev/rules.d



4. load the module: this results in the module loading every time the system boots, as required

sudo modprobe xdma


5. if it is necessary to unload the module (eg to recompile it)

rmmod -s xdma


6. to buld the tools for testing:

cd ~/github/saturn/linuxdriver/tools
make
