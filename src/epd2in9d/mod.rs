//! A simple Driver for the Waveshare 2.9" D E-Ink Display via SPI
//!
//!
//! 参考[Waveshare](https://www.waveshare.net/wiki/2.9inch_e-Paper_HAT_%28D%29)的文档/例程进行构建
//!
//! Specification: https://www.waveshare.net/w/upload/b/b5/2.9inch_e-Paper_%28D%29_Specification.pdf

use core::fmt::{Debug, Display};
use core::slice::from_raw_parts;

use embedded_hal::digital::{InputPin, OutputPin};
use embedded_hal_async::{digital::Wait, spi::SpiDevice};

use crate::traits::{InternalWiAdditions, RefreshLut, WaveshareDisplay};
use crate::{interface::DisplayInterface, prelude::ErrorKind, traits::ErrorType};

//The Lookup Tables for the Display
mod constants;
use crate::epd2in9d::constants::*;

/// Width of Epd2in9d in pixels
pub const WIDTH: u32 = 128;
/// Height of Epd2in9d in pixels
pub const HEIGHT: u32 = 296;
/// EPD_ARRAY of Epd2in9d in pixels
/// WIDTH / 8 * HEIGHT
pub const EPD_ARRAY: u32 = 4736;
/// Default Background Color (white)
pub const DEFAULT_BACKGROUND_COLOR: Color = Color::Black;
const IS_BUSY_LOW: bool = false;
const SINGLE_BYTE_WRITE: bool = true;

use crate::color::Color;

pub(crate) mod command;
use self::command::Command;
use crate::buffer_len;

/// Display with Fullsize buffer for use with the 2in9 EPD D
#[cfg(feature = "graphics")]
pub type Display2in9d = crate::graphics::Display<
    WIDTH,
    HEIGHT,
    false,
    { buffer_len(WIDTH as usize, HEIGHT as usize) },
    Color,
>;

/// Epd2in9d driver
///
pub struct Epd2in9d<'a, SPI, BUSY, DC, RST> {
    /// SPI
    interface: DisplayInterface<SPI, BUSY, DC, RST, SINGLE_BYTE_WRITE>,
    /// Color
    // background_color: Color,
    color: Color,
    /// Refresh LUT
    refresh: RefreshLut,
    // Storing old data for partial refreshes
    old_data: &'a [u8],
    // 标记是否局刷的状态
    is_partial_refresh: bool,
}

impl<'a, SPI, BUSY, DC, RST> ErrorType<SPI, BUSY, DC, RST> for Epd2in9d<'a, SPI, BUSY, DC, RST>
where
    SPI: SpiDevice,
    SPI::Error: Copy + Debug + Display,
    BUSY: InputPin + Wait,
    BUSY::Error: Copy + Debug + Display,
    DC: OutputPin,
    DC::Error: Copy + Debug + Display,
    RST: OutputPin,
    RST::Error: Copy + Debug + Display,
{
    type Error = ErrorKind<SPI, BUSY, DC, RST>;
}

impl<SPI, BUSY, DC, RST> InternalWiAdditions<SPI, BUSY, DC, RST>
    for Epd2in9d<'_, SPI, BUSY, DC, RST>
where
    SPI: SpiDevice,
    SPI::Error: Copy + Debug + Display,
    BUSY: InputPin + Wait,
    BUSY::Error: Copy + Debug + Display,
    DC: OutputPin,
    DC::Error: Copy + Debug + Display,
    RST: OutputPin,
    RST::Error: Copy + Debug + Display,
{
    async fn init(&mut self, spi: &mut SPI) -> Result<(), Self::Error> {
        self.interface.reset(spi, 10_000, 2_000).await?;

        //panel setting
        //LUT from OTP，KW-BF   KWR-AF	BWROTP 0f	BWOTP 1f
        self.interface
            .cmd_with_data(spi, Command::PanelSetting, &[0x1f, 0x0D])
            .await?;

        //resolution setting
        self.interface
            .cmd_with_data(spi, Command::ResolutionSetting, &[0x80, 0x01, 0x28])
            .await?;

        self.interface.cmd(spi, Command::PowerOn).await?;
        self.wait_until_idle(spi).await?;

        //VCOM AND DATA INTERVAL SETTING
        self.interface
            .cmd_with_data(spi, Command::VcomAndDataIntervalSetting, &[0x97])
            .await?;

        Ok(())
    }
}

impl<SPI, BUSY, DC, RST> WaveshareDisplay<SPI, BUSY, DC, RST> for Epd2in9d<'_, SPI, BUSY, DC, RST>
where
    SPI: SpiDevice,
    SPI::Error: Copy + Debug + Display,
    BUSY: InputPin + Wait,
    BUSY::Error: Copy + Debug + Display,
    DC: OutputPin,
    DC::Error: Copy + Debug + Display,
    RST: OutputPin,
    RST::Error: Copy + Debug + Display,
{
    type DisplayColor = Color;
    async fn new(
        spi: &mut SPI,
        busy: BUSY,
        dc: DC,
        rst: RST,
        delay_us: Option<u32>,
    ) -> Result<Self, Self::Error> {
        let interface = DisplayInterface::new(busy, dc, rst, delay_us);
        let color = DEFAULT_BACKGROUND_COLOR;
        let old_data: &[u8] = &[];
        let is_partial_refresh = false;

        let mut epd = Epd2in9d {
            interface,
            color,
            refresh: RefreshLut::Full,
            old_data,
            is_partial_refresh,
        };

        epd.init(spi).await?;

        Ok(epd)
    }

    async fn sleep(&mut self, spi: &mut SPI) -> Result<(), Self::Error> {
        self.is_partial_refresh = false;
        self.interface
            .cmd_with_data(spi, Command::VcomAndDataIntervalSetting, &[0xf7])
            .await?;
        self.interface.cmd(spi, Command::PowerOff).await?;
        self.wait_until_idle(spi).await?;
        self.interface.delay(spi, 100_000).await?;
        self.interface
            .cmd_with_data(spi, Command::DeepSleep, &[0xA5])
            .await?;

        Ok(())
    }

    async fn wake_up(&mut self, spi: &mut SPI) -> Result<(), Self::Error> {
        self.init(spi).await?;
        Ok(())
    }

    fn set_background_color(&mut self, background_color: Color) {
        self.color = background_color;
    }

    fn background_color(&self) -> &Color {
        &self.color
    }

    fn width(&self) -> u32 {
        WIDTH
    }

    fn height(&self) -> u32 {
        HEIGHT
    }

    // Corresponds to the Display function.
    // Used to write the data to be displayed to the screen SRAM.
    async fn update_frame(&mut self, spi: &mut SPI, buffer: &[u8]) -> Result<(), Self::Error> {
        if self.is_partial_refresh {
            // Modify local refresh status if full refresh is performed.
            self.is_partial_refresh = false;
        }
        self.wait_until_idle(spi).await?;

        self.interface
            .cmd(spi, Command::DataStartTransmission1)
            .await?;
        self.interface.data_x_times(spi, 0xFF, EPD_ARRAY).await?;

        self.interface
            .cmd_with_data(spi, Command::DataStartTransmission2, buffer)
            .await?;
        self.old_data = unsafe { from_raw_parts(buffer.as_ptr(), buffer.len()) };
        Ok(())
    }

    // 这个是DisplayPart
    // Partial refresh write address and data
    async fn update_partial_frame(
        &mut self,
        spi: &mut SPI,
        buffer: &[u8],
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Result<(), Self::Error> {
        if !self.is_partial_refresh {
            // Initialize only on first call
            self.set_part_reg(spi).await?;
            self.is_partial_refresh = true;
        }
        self.interface.cmd(spi, Command::PartialIn).await?;

        self.interface.cmd(spi, Command::PartialWindow).await?;
        self.interface.data(spi, &[(x - x % 8) as u8]).await?;
        self.interface
            .data(spi, &[(((x - x % 8) + width - 1) - 1) as u8])
            .await?;
        self.interface.data(spi, &[(y / 256) as u8]).await?;
        self.interface.data(spi, &[(y % 256) as u8]).await?;
        self.interface
            .data(spi, &[((y + height - 1) / 256) as u8])
            .await?;
        self.interface
            .data(spi, &[((y + height - 1) % 256 - 1) as u8])
            .await?;
        self.interface.data(spi, &[0x28]).await?;

        self.interface
            .cmd_with_data(spi, Command::DataStartTransmission1, self.old_data)
            .await?;

        self.interface
            .cmd_with_data(spi, Command::DataStartTransmission2, buffer)
            .await?;
        self.old_data = unsafe { from_raw_parts(buffer.as_ptr(), buffer.len()) };

        Ok(())
    }

    /// actually is the "Turn on Display" sequence
    async fn display_frame(&mut self, spi: &mut SPI) -> Result<(), Self::Error> {
        self.interface.cmd(spi, Command::DisplayRefresh).await?;
        self.interface.delay(spi, 1_000).await?;
        self.wait_until_idle(spi).await?;
        Ok(())
    }

    async fn update_and_display_frame(
        &mut self,
        spi: &mut SPI,
        buffer: &[u8],
    ) -> Result<(), Self::Error> {
        self.update_frame(spi, buffer).await?;
        self.display_frame(spi).await?;
        Ok(())
    }

    async fn clear_frame(&mut self, spi: &mut SPI) -> Result<(), Self::Error> {
        self.interface
            .cmd(spi, Command::DataStartTransmission1)
            .await?;
        self.interface.data_x_times(spi, 0x00, EPD_ARRAY).await?;

        self.interface
            .cmd(spi, Command::DataStartTransmission2)
            .await?;
        self.interface.data_x_times(spi, 0xFF, EPD_ARRAY).await?;

        self.display_frame(spi).await?;

        Ok(())
    }

    async fn set_lut(
        &mut self,
        spi: &mut SPI,
        refresh_rate: Option<RefreshLut>,
    ) -> Result<(), Self::Error> {
        if let Some(refresh_lut) = refresh_rate {
            self.refresh = refresh_lut;
        }
        self.set_lut_helper(spi, &LUT_VCOM1, &LUT_WW1, &LUT_BW1, &LUT_WB1, &LUT_BB1)
            .await?;

        Ok(())
    }

    async fn wait_until_idle(&mut self, spi: &mut SPI) -> Result<(), Self::Error> {
        self.interface.wait_until_idle(spi, IS_BUSY_LOW).await
    }
}

impl<SPI, BUSY, DC, RST> Epd2in9d<'_, SPI, BUSY, DC, RST>
where
    SPI: SpiDevice,
    SPI::Error: Copy + Debug + Display,
    BUSY: InputPin + Wait,
    BUSY::Error: Copy + Debug + Display,
    DC: OutputPin,
    DC::Error: Copy + Debug + Display,
    RST: OutputPin,
    RST::Error: Copy + Debug + Display,
{
    /// Wake Up Screen
    ///
    /// After the screen sleeps, it enters deep sleep mode. If you need to refresh the screen while in deep sleep mode, you must first execute awaken().
    /// Wake the screen.
    // fn awaken(&mut self, spi: &mut SPI) -> Result<(), Self::Error> {
    //     // reset the device
    //     self.interface.reset(spi, 20_000, 2_000)?;
    //     self.wait_until_idle(spi, delay)?;

    //     // panel setting
    //     // LUT from OTP，KW-BF   KWR-AF	BWROTP 0f	BWOTP 1f
    //     self.interface
    //         .cmd_with_data(spi, Command::PanelSetting, &[0x1f])?;

    //     // resolution setting
    //     self.interface
    //         .cmd_with_data(spi, Command::ResolutionSetting, &[0x80, 0x01, 0x28])?;

    //     // VCOM AND DATA INTERVAL SETTING
    //     self.interface
    //         .cmd(spi, Command::VcomAndDataIntervalSetting)?;
    //     Ok(())
    // }

    async fn set_part_reg(
        &mut self,
        spi: &mut SPI,
    ) -> Result<(), <Self as ErrorType<SPI, BUSY, DC, RST>>::Error> {
        // Reset the EPD driver circuit
        //TODO: 这里在微雪的例程中反复刷新了3次，后面有显示问题再进行修改
        self.interface.reset(spi, 10_000, 2_000).await?;

        // Power settings
        //TODO: The data in the document is [0x03,0x00,0x2b,0x2b,0x09].
        self.interface
            .cmd_with_data(spi, Command::PowerSetting, &[0x03, 0x00, 0x2b, 0x2b, 0x03])
            .await?;

        // Soft start
        self.interface
            .cmd_with_data(spi, Command::BoosterSoftStart, &[0x17, 0x17, 0x17])
            .await?;

        // Panel settings
        self.interface
            .cmd_with_data(spi, Command::PanelSetting, &[0xbf, 0x0D])
            .await?;

        // Setting the refresh rate
        // 3a 100HZ | 29 150Hz | 39 200HZ | 31 171HZ
        // 3a is used in the example
        self.interface
            .cmd_with_data(spi, Command::PllControl, &[0x3C])
            .await?;

        // Resolution Settings
        self.interface
            .cmd_with_data(spi, Command::ResolutionSetting, &[0x80, 0x01, 0x28])
            .await?;

        // vcom_DC settings
        self.interface
            .cmd_with_data(spi, Command::VcmDcSetting, &[0x12])
            .await?;

        self.set_lut(spi, None).await?;

        // Power on
        // self.interface.cmd_with_data(
        //     spi,
        //     Command::PowerOn,
        //     &[0x04],
        // );
        self.interface.cmd(spi, Command::PowerOn).await?;

        // Get the BUSY level, high to continue, low to wait for the screen to respond.
        //TODO: This is the recommended step in the documentation, but I've ignored it since I've seen other screens that don't wait.
        self.wait_until_idle(spi).await?;

        // vcom and data interval settings
        // self.interface
        //     .cmd_with_data(spi, Command::VcomAndDataIntervalSetting, &[0x97])?;

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn set_lut_helper(
        &mut self,
        spi: &mut SPI,
        lut_vcom: &[u8],
        lut_ww: &[u8],
        lut_bw: &[u8],
        lut_wb: &[u8],
        lut_bb: &[u8],
    ) -> Result<(), <Self as ErrorType<SPI, BUSY, DC, RST>>::Error> {
        // LUT VCOM
        self.interface
            .cmd_with_data(spi, Command::LutForVcom, lut_vcom)
            .await?;

        // LUT WHITE to WHITE
        self.interface
            .cmd_with_data(spi, Command::LutWhiteToWhite, lut_ww)
            .await?;

        // LUT BLACK to WHITE
        self.interface
            .cmd_with_data(spi, Command::LutBlackToWhite, lut_bw)
            .await?;

        // LUT WHITE to BLACK
        self.interface
            .cmd_with_data(spi, Command::LutWhiteToBlack, lut_wb)
            .await?;

        // LUT BLACK to BLACK
        self.interface
            .cmd_with_data(spi, Command::LutBlackToBlack, lut_bb)
            .await?;
        Ok(())
    }
}
