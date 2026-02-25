/* Copyright (C)
* 2024 - Laurence Barker G8NJJ
*
*   This program is free software: you can redistribute it and/or modify
*   it under the terms of the GNU General Public License as published by
*   the Free Software Foundation, either version 3 of the License, or
*   (at your option) any later version.
*
*   This program is distributed in the hope that it will be useful,
*   but WITHOUT ANY WARRANTY; without even the implied warranty of
*   MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
*   GNU General Public License for more details.
*
*   You should have received a copy of the GNU General Public License
*   along with this program.  If not, see <https://www.gnu.org/licenses/>.
*
*/
#include <string.h>
#include <unistd.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <linux/i2c-dev.h>
#include <i2c/smbus.h>
#include <sys/ioctl.h>
#include <fcntl.h>
#include <stdint.h>
#include <pthread.h>

#include "i2cdriver.h"
extern int i2c_fd;                                  // file reference
static pthread_mutex_t g_i2c_bus_mutex = PTHREAD_MUTEX_INITIALIZER;  // serialize shared I2C bus access

//
// open and configure shared I2C device file
//
int i2c_open_device(const char* device_path, int slave_addr)
{
  int fd;

  fd = open(device_path, O_RDWR);
  if(fd < 0)
    return -1;

  if(ioctl(fd, I2C_SLAVE, slave_addr) < 0)
  {
    close(fd);
    return -1;
  }

  pthread_mutex_lock(&g_i2c_bus_mutex);
  if(i2c_fd >= 0)
    close(i2c_fd);
  i2c_fd = fd;
  pthread_mutex_unlock(&g_i2c_bus_mutex);
  return 0;
}

//
// close shared I2C device file
//
void i2c_close_device(void)
{
  pthread_mutex_lock(&g_i2c_bus_mutex);
  if(i2c_fd >= 0)
  {
    close(i2c_fd);
    i2c_fd = -1;
  }
  pthread_mutex_unlock(&g_i2c_bus_mutex);
}


//
// 8 bit write
//
int i2c_write_byte_data(uint8_t reg, uint8_t data) 
{
  int rc;

  pthread_mutex_lock(&g_i2c_bus_mutex);
  if(i2c_fd < 0)
    rc = -1;
  else if ((rc = i2c_smbus_write_byte_data(i2c_fd, reg, data & 0xFF)) < 0) 
  {
    printf("%s: write i2c failed: addr=%02X\n", __FUNCTION__, reg);
  }
  pthread_mutex_unlock(&g_i2c_bus_mutex);

  return rc;
}

//
// 16 bit write
//
int i2c_write_word_data(uint8_t reg, uint16_t data)
{
  int rc;

  pthread_mutex_lock(&g_i2c_bus_mutex);
  if(i2c_fd < 0)
    rc = -1;
  else if ((rc = i2c_smbus_write_word_data(i2c_fd, reg, data & 0xFFFF)) < 0) 
  {
    printf("%s: 16 bit write i2c failed: addr=%02X\n", __FUNCTION__, reg);
  }
  pthread_mutex_unlock(&g_i2c_bus_mutex);

  return rc;
}



//
// 8 bit read
// used to detect presence of an MCP23017 in tests for G2V1 panel presence
//
uint8_t i2c_read_byte_data(uint8_t reg, bool *error) 
{
  int32_t data;

  *error = false;
  pthread_mutex_lock(&g_i2c_bus_mutex);
  if(i2c_fd < 0)
    data = -1;
  else
    data = i2c_smbus_read_byte_data(i2c_fd, reg);
  pthread_mutex_unlock(&g_i2c_bus_mutex);
  if(data < 0)
  {
    *error = true;
    printf("I2C register not found, code=%d\n", data);
  }
  return (uint8_t) (data & 0xFF);
}


//
// 16 bit read 
//
uint16_t i2c_read_word_data(uint8_t reg, bool *error) 
{
  int32_t data;


  *error = false;
  pthread_mutex_lock(&g_i2c_bus_mutex);
  if(i2c_fd < 0)
    data = -1;
  else
    data = i2c_smbus_read_word_data(i2c_fd, reg);
  pthread_mutex_unlock(&g_i2c_bus_mutex);
  if(data < 0)
  {
    *error = true;
    printf("I2C register not found, code=%d\n", data);
  }
  return (uint16_t) (data & 0xFFFF);
}
