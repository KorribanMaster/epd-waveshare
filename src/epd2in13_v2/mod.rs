//! A Driver for the Waveshare 2.13" E-Ink Display (V2 and V3) via SPI
//!
//! # References V2
//!
//! - [Waveshare product page](https://www.waveshare.com/wiki/2.13inch_e-Paper_HAT)
//! - [Waveshare C driver](https://github.com/waveshare/e-Paper/blob/master/RaspberryPi%26JetsonNano/c/lib/e-Paper/EPD_2in13_V2.c)
//! - [Waveshare Python driver](https://github.com/waveshare/e-Paper/blob/master/RaspberryPi%26JetsonNano/python/lib/waveshare_epd/epd2in13_V2.py)
//! - [Controller Datasheet SS1780](http://www.e-paper-display.com/download_detail/downloadsId=682.html)
//!
//! # References V3
//!
//! - [Waveshare product page](https://www.waveshare.com/wiki/2.13inch_e-Paper_HAT)
//! - [Waveshare C driver](https://github.com/waveshare/e-Paper/blob/master/RaspberryPi_JetsonNano/c/lib/e-Paper/EPD_2in13_V3.c)
//! - [Waveshare Python driver](https://github.com/waveshare/e-Paper/blob/master/RaspberryPi_JetsonNano/python/lib/waveshare_epd/epd2in9b_V3.py)
//! - [Controller Datasheet SS1780](http://www.e-paper-display.com/download_detail/downloadsId=682.html)
//!
use core::fmt::{Debug, Display};
use embedded_hal::digital::{InputPin, OutputPin};
use embedded_hal_async::{digital::Wait, spi::SpiDevice};

use crate::buffer_len;
use crate::color::Color;
use crate::error::ErrorKind;
use crate::interface::DisplayInterface;
use crate::traits::{ErrorType, InternalWiAdditions, RefreshLut, WaveshareDisplay};

pub(crate) mod command;
use self::command::{
    BorderWaveForm, BorderWaveFormFixLevel, BorderWaveFormGs, BorderWaveFormVbd, Command,
    DataEntryModeDir, DataEntryModeIncr, DeepSleepMode, DisplayUpdateControl2, DriverOutput,
    GateDrivingVoltage, I32Ext, SourceDrivingVoltage, Vcom,
};

pub(crate) mod constants;

use self::constants::{LUT_FULL_UPDATE, LUT_PARTIAL_UPDATE};
#[cfg(all(feature = "epd2in13_v2", feature = "epd2in13_v3"))]
compile_error!(
    "feature \"epd2in13_v2\" and feature \"epd2in13_v3\" cannot be enabled at the same time"
);
#[cfg(not(any(feature = "epd2in13_v2", feature = "epd2in13_v3")))]
compile_error!(
    "One of feature \"epd2in13_v2\" and feature \"epd2in13_v3\" needs to be enabled as a feature"
);

/// Full size buffer for use with the 2in13 v2 and v3 EPD
#[cfg(feature = "graphics")]
pub type Display2in13 = crate::graphics::Display<
    WIDTH,
    HEIGHT,
    false,
    { buffer_len(WIDTH as usize, HEIGHT as usize) },
    Color,
>;

/// Width of the display.
pub const WIDTH: u32 = 122;

/// Height of the display
pub const HEIGHT: u32 = 250;

/// Default Background Color
pub const DEFAULT_BACKGROUND_COLOR: Color = Color::White;
const IS_BUSY_LOW: bool = false;
const SINGLE_BYTE_WRITE: bool = true;

/// Epd2in13 (V2 & V3) driver
///
/// To use this driver for V2 of the display, feature \"epd2in13_v3\" needs to be disabled and feature \"epd2in13_v2\" enabled.
pub struct Epd2in13<SPI, BUSY, DC, RST> {
    /// Connection Interface
    interface: DisplayInterface<SPI, BUSY, DC, RST, SINGLE_BYTE_WRITE>,

    sleep_mode: DeepSleepMode,

    /// Background Color
    background_color: Color,
    refresh: RefreshLut,
}

impl<SPI, BUSY, DC, RST> ErrorType<SPI, BUSY, DC, RST> for Epd2in13<SPI, BUSY, DC, RST>
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
    for Epd2in13<SPI, BUSY, DC, RST>
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
        // HW reset
        self.interface.reset(spi, 10_000, 10_000).await?;

        if self.refresh == RefreshLut::Quick {
            self.set_vcom_register(spi, (-9).vcom()).await?;
            self.wait_until_idle(spi).await?;

            self.set_lut(spi, Some(self.refresh)).await?;

            // Python code does this, not sure why
            // self.cmd_with_data(spi, Command::WriteOtpSelection, &[0, 0, 0, 0, 0x40, 0, 0])?;

            // During partial update, clock/analog are not disabled between 2
            // updates.
            self.set_display_update_control_2(
                spi,
                DisplayUpdateControl2::new().enable_analog().enable_clock(),
            )
            .await?;
            self.command(spi, Command::MasterActivation).await?;
            self.wait_until_idle(spi).await?;

            self.set_border_waveform(
                spi,
                BorderWaveForm {
                    vbd: BorderWaveFormVbd::Gs,
                    fix_level: BorderWaveFormFixLevel::Vss,
                    gs_trans: BorderWaveFormGs::Lut1,
                },
            )
            .await?;
        } else {
            self.wait_until_idle(spi).await?;
            self.command(spi, Command::SwReset).await?;
            self.wait_until_idle(spi).await?;

            self.set_driver_output(
                spi,
                DriverOutput {
                    scan_is_linear: true,
                    scan_g0_is_first: true,
                    scan_dir_incr: true,
                    width: (HEIGHT - 1) as u16,
                },
            )
            .await?;

            // These 2 are the reset values
            self.set_dummy_line_period(spi, 0x30).await?;
            self.set_gate_scan_start_position(spi, 0).await?;

            self.set_data_entry_mode(spi, DataEntryModeIncr::XIncrYIncr, DataEntryModeDir::XDir)
                .await?;

            // Use simple X/Y auto increase
            self.set_ram_area(spi, 0, 0, WIDTH - 1, HEIGHT - 1).await?;
            self.set_ram_address_counters(spi, 0, 0).await?;

            self.set_border_waveform(
                spi,
                BorderWaveForm {
                    vbd: BorderWaveFormVbd::Gs,
                    fix_level: BorderWaveFormFixLevel::Vss,
                    gs_trans: BorderWaveFormGs::Lut3,
                },
            )
            .await?;

            self.set_vcom_register(spi, (-21).vcom()).await?;

            self.set_gate_driving_voltage(spi, 190.gate_driving_decivolt())
                .await?;
            self.set_source_driving_voltage(
                spi,
                150.source_driving_decivolt(),
                50.source_driving_decivolt(),
                (-150).source_driving_decivolt(),
            )
            .await?;

            self.set_gate_line_width(spi, 10).await?;

            self.set_lut(spi, Some(self.refresh)).await?;
        }

        self.wait_until_idle(spi).await?;
        Ok(())
    }
}

impl<SPI, BUSY, DC, RST> WaveshareDisplay<SPI, BUSY, DC, RST> for Epd2in13<SPI, BUSY, DC, RST>
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
        let mut epd = Epd2in13 {
            interface: DisplayInterface::new(busy, dc, rst, delay_us),
            sleep_mode: DeepSleepMode::Mode1,
            background_color: DEFAULT_BACKGROUND_COLOR,
            refresh: RefreshLut::Full,
        };

        epd.init(spi).await?;
        Ok(epd)
    }

    async fn wake_up(&mut self, spi: &mut SPI) -> Result<(), Self::Error> {
        self.init(spi).await
    }

    async fn sleep(&mut self, spi: &mut SPI) -> Result<(), Self::Error> {
        self.wait_until_idle(spi).await?;

        // All sample code enables and disables analog/clocks...
        self.set_display_update_control_2(
            spi,
            DisplayUpdateControl2::new()
                .enable_analog()
                .enable_clock()
                .disable_analog()
                .disable_clock(),
        )
        .await?;
        self.command(spi, Command::MasterActivation).await?;

        self.set_sleep_mode(spi, self.sleep_mode).await?;
        Ok(())
    }

    async fn update_frame(&mut self, spi: &mut SPI, buffer: &[u8]) -> Result<(), Self::Error> {
        assert!(buffer.len() == buffer_len(WIDTH as usize, HEIGHT as usize));
        self.set_ram_area(spi, 0, 0, WIDTH - 1, HEIGHT - 1).await?;
        self.set_ram_address_counters(spi, 0, 0).await?;

        self.cmd_with_data(spi, Command::WriteRam, buffer).await?;

        if self.refresh == RefreshLut::Full {
            // Always keep the base buffer equal to current if not doing partial refresh.
            self.set_ram_area(spi, 0, 0, WIDTH - 1, HEIGHT - 1).await?;
            self.set_ram_address_counters(spi, 0, 0).await?;

            self.cmd_with_data(spi, Command::WriteRamRed, buffer)
                .await?;
        }
        Ok(())
    }

    /// Updating only a part of the frame is not supported when using the
    /// partial refresh feature. The function will panic if called when set to
    /// use partial refresh.
    async fn update_partial_frame(
        &mut self,
        spi: &mut SPI,
        buffer: &[u8],
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Result<(), Self::Error> {
        assert!((width * height / 8) as usize == buffer.len());

        // This should not be used when doing partial refresh. The RAM_RED must
        // be updated with the last buffer having been displayed. Doing partial
        // update directly in RAM makes this update impossible (we can't read
        // RAM content). Using this function will most probably make the actual
        // display incorrect as the controler will compare with something
        // incorrect.
        assert!(self.refresh == RefreshLut::Full);

        self.set_ram_area(spi, x, y, x + width, y + height).await?;
        self.set_ram_address_counters(spi, x, y).await?;

        self.cmd_with_data(spi, Command::WriteRam, buffer).await?;

        if self.refresh == RefreshLut::Full {
            // Always keep the base buffer equals to current if not doing partial refresh.
            self.set_ram_area(spi, x, y, x + width, y + height).await?;
            self.set_ram_address_counters(spi, x, y).await?;

            self.cmd_with_data(spi, Command::WriteRamRed, buffer)
                .await?;
        }

        Ok(())
    }

    /// Never use directly this function when using partial refresh, or also
    /// keep the base buffer in syncd using `set_partial_base_buffer` function.
    async fn display_frame(&mut self, spi: &mut SPI) -> Result<(), Self::Error> {
        if self.refresh == RefreshLut::Full {
            self.set_display_update_control_2(
                spi,
                DisplayUpdateControl2::new()
                    .enable_clock()
                    .enable_analog()
                    .display()
                    .disable_analog()
                    .disable_clock(),
            )
            .await?;
        } else {
            self.set_display_update_control_2(spi, DisplayUpdateControl2::new().display())
                .await?;
        }
        self.command(spi, Command::MasterActivation).await?;
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

        if self.refresh == RefreshLut::Quick {
            self.set_partial_base_buffer(spi, buffer).await?;
        }
        Ok(())
    }

    async fn clear_frame(&mut self, spi: &mut SPI) -> Result<(), Self::Error> {
        let color = self.background_color.get_byte_value();

        self.set_ram_area(spi, 0, 0, WIDTH - 1, HEIGHT - 1).await?;
        self.set_ram_address_counters(spi, 0, 0).await?;

        self.command(spi, Command::WriteRam).await?;
        self.interface
            .data_x_times(
                spi,
                color,
                buffer_len(WIDTH as usize, HEIGHT as usize) as u32,
            )
            .await?;

        // Always keep the base buffer equals to current if not doing partial refresh.
        if self.refresh == RefreshLut::Full {
            self.set_ram_area(spi, 0, 0, WIDTH - 1, HEIGHT - 1).await?;
            self.set_ram_address_counters(spi, 0, 0).await?;

            self.command(spi, Command::WriteRamRed).await?;
            self.interface
                .data_x_times(
                    spi,
                    color,
                    buffer_len(WIDTH as usize, HEIGHT as usize) as u32,
                )
                .await?;
        }
        Ok(())
    }

    fn set_background_color(&mut self, background_color: Color) {
        self.background_color = background_color;
    }

    fn background_color(&self) -> &Color {
        &self.background_color
    }

    fn width(&self) -> u32 {
        WIDTH
    }

    fn height(&self) -> u32 {
        HEIGHT
    }

    async fn set_lut(
        &mut self,
        spi: &mut SPI,
        refresh_rate: Option<RefreshLut>,
    ) -> Result<(), Self::Error> {
        let buffer = match refresh_rate {
            Some(RefreshLut::Full) | None => &LUT_FULL_UPDATE,
            Some(RefreshLut::Quick) => &LUT_PARTIAL_UPDATE,
        };

        self.cmd_with_data(spi, Command::WriteLutRegister, buffer)
            .await
    }

    async fn wait_until_idle(&mut self, spi: &mut SPI) -> Result<(), Self::Error> {
        self.interface.wait_until_idle(spi, IS_BUSY_LOW).await?;
        Ok(())
    }
}

impl<SPI, BUSY, DC, RST> Epd2in13<SPI, BUSY, DC, RST>
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
    /// When using partial refresh, the controller uses the provided buffer for
    /// comparison with new buffer.
    pub async fn set_partial_base_buffer(
        &mut self,
        spi: &mut SPI,
        buffer: &[u8],
    ) -> Result<(), <Self as ErrorType<SPI, BUSY, DC, RST>>::Error> {
        assert!(buffer_len(WIDTH as usize, HEIGHT as usize) == buffer.len());
        self.set_ram_area(spi, 0, 0, WIDTH - 1, HEIGHT - 1).await?;
        self.set_ram_address_counters(spi, 0, 0).await?;

        self.cmd_with_data(spi, Command::WriteRamRed, buffer)
            .await?;
        Ok(())
    }

    /// Selects which sleep mode will be used when triggering the deep sleep.
    pub fn set_deep_sleep_mode(&mut self, mode: DeepSleepMode) {
        self.sleep_mode = mode;
    }

    /// Sets the refresh mode. When changing mode, the screen will be
    /// re-initialized accordingly.
    pub async fn set_refresh(
        &mut self,
        spi: &mut SPI,
        refresh: RefreshLut,
    ) -> Result<(), <Self as ErrorType<SPI, BUSY, DC, RST>>::Error> {
        if self.refresh != refresh {
            self.refresh = refresh;
            self.init(spi).await?;
        }
        Ok(())
    }

    async fn set_gate_scan_start_position(
        &mut self,
        spi: &mut SPI,
        start: u16,
    ) -> Result<(), <Self as ErrorType<SPI, BUSY, DC, RST>>::Error> {
        assert!(start <= 295);
        self.cmd_with_data(
            spi,
            Command::GateScanStartPosition,
            &[(start & 0xFF) as u8, ((start >> 8) & 0x1) as u8],
        )
        .await
    }

    async fn set_border_waveform(
        &mut self,
        spi: &mut SPI,
        borderwaveform: BorderWaveForm,
    ) -> Result<(), <Self as ErrorType<SPI, BUSY, DC, RST>>::Error> {
        self.cmd_with_data(
            spi,
            Command::BorderWaveformControl,
            &[borderwaveform.to_u8()],
        )
        .await
    }

    async fn set_vcom_register(
        &mut self,
        spi: &mut SPI,
        vcom: Vcom,
    ) -> Result<(), <Self as ErrorType<SPI, BUSY, DC, RST>>::Error> {
        self.cmd_with_data(spi, Command::WriteVcomRegister, &[vcom.0])
            .await
    }

    async fn set_gate_driving_voltage(
        &mut self,
        spi: &mut SPI,
        voltage: GateDrivingVoltage,
    ) -> Result<(), <Self as ErrorType<SPI, BUSY, DC, RST>>::Error> {
        self.cmd_with_data(spi, Command::GateDrivingVoltageCtrl, &[voltage.0])
            .await
    }

    async fn set_dummy_line_period(
        &mut self,
        spi: &mut SPI,
        number_of_lines: u8,
    ) -> Result<(), <Self as ErrorType<SPI, BUSY, DC, RST>>::Error> {
        assert!(number_of_lines <= 127);
        self.cmd_with_data(spi, Command::SetDummyLinePeriod, &[number_of_lines])
            .await
    }

    async fn set_gate_line_width(
        &mut self,
        spi: &mut SPI,
        width: u8,
    ) -> Result<(), <Self as ErrorType<SPI, BUSY, DC, RST>>::Error> {
        self.cmd_with_data(spi, Command::SetGateLineWidth, &[width & 0x0F])
            .await
    }

    /// Sets the source driving voltage value
    async fn set_source_driving_voltage(
        &mut self,
        spi: &mut SPI,
        vsh1: SourceDrivingVoltage,
        vsh2: SourceDrivingVoltage,
        vsl: SourceDrivingVoltage,
    ) -> Result<(), <Self as ErrorType<SPI, BUSY, DC, RST>>::Error> {
        self.cmd_with_data(
            spi,
            Command::SourceDrivingVoltageCtrl,
            &[vsh1.0, vsh2.0, vsl.0],
        )
        .await
    }

    /// Prepare the actions that the next master activation command will
    /// trigger.
    async fn set_display_update_control_2(
        &mut self,
        spi: &mut SPI,
        value: DisplayUpdateControl2,
    ) -> Result<(), <Self as ErrorType<SPI, BUSY, DC, RST>>::Error> {
        self.cmd_with_data(spi, Command::DisplayUpdateControl2, &[value.0])
            .await
    }

    /// Triggers the deep sleep mode
    async fn set_sleep_mode(
        &mut self,
        spi: &mut SPI,
        mode: DeepSleepMode,
    ) -> Result<(), <Self as ErrorType<SPI, BUSY, DC, RST>>::Error> {
        self.cmd_with_data(spi, Command::DeepSleepMode, &[mode as u8])
            .await
    }

    async fn set_driver_output(
        &mut self,
        spi: &mut SPI,
        output: DriverOutput,
    ) -> Result<(), <Self as ErrorType<SPI, BUSY, DC, RST>>::Error> {
        self.cmd_with_data(spi, Command::DriverOutputControl, &output.to_bytes())
            .await
    }

    /// Sets the data entry mode (ie. how X and Y positions changes when writing
    /// data to RAM)
    async fn set_data_entry_mode(
        &mut self,
        spi: &mut SPI,
        counter_incr_mode: DataEntryModeIncr,
        counter_direction: DataEntryModeDir,
    ) -> Result<(), <Self as ErrorType<SPI, BUSY, DC, RST>>::Error> {
        let mode = counter_incr_mode as u8 | counter_direction as u8;
        self.cmd_with_data(spi, Command::DataEntryModeSetting, &[mode])
            .await
    }

    /// Sets both X and Y pixels ranges
    async fn set_ram_area(
        &mut self,
        spi: &mut SPI,
        start_x: u32,
        start_y: u32,
        end_x: u32,
        end_y: u32,
    ) -> Result<(), <Self as ErrorType<SPI, BUSY, DC, RST>>::Error> {
        self.cmd_with_data(
            spi,
            Command::SetRamXAddressStartEndPosition,
            &[(start_x >> 3) as u8, (end_x >> 3) as u8],
        )
        .await?;

        self.cmd_with_data(
            spi,
            Command::SetRamYAddressStartEndPosition,
            &[
                start_y as u8,
                (start_y >> 8) as u8,
                end_y as u8,
                (end_y >> 8) as u8,
            ],
        )
        .await
    }

    /// Sets both X and Y pixels counters when writing data to RAM
    async fn set_ram_address_counters(
        &mut self,
        spi: &mut SPI,
        x: u32,
        y: u32,
    ) -> Result<(), <Self as ErrorType<SPI, BUSY, DC, RST>>::Error> {
        self.wait_until_idle(spi).await?;
        self.cmd_with_data(spi, Command::SetRamXAddressCounter, &[(x >> 3) as u8])
            .await?;

        self.cmd_with_data(
            spi,
            Command::SetRamYAddressCounter,
            &[y as u8, (y >> 8) as u8],
        )
        .await?;
        Ok(())
    }

    async fn command(
        &mut self,
        spi: &mut SPI,
        command: Command,
    ) -> Result<(), <Self as ErrorType<SPI, BUSY, DC, RST>>::Error> {
        self.interface.cmd(spi, command).await
    }

    async fn cmd_with_data(
        &mut self,
        spi: &mut SPI,
        command: Command,
        data: &[u8],
    ) -> Result<(), <Self as ErrorType<SPI, BUSY, DC, RST>>::Error> {
        self.interface.cmd_with_data(spi, command, data).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epd_size() {
        assert_eq!(WIDTH, 122);
        assert_eq!(HEIGHT, 250);
        assert_eq!(DEFAULT_BACKGROUND_COLOR, Color::White);
    }
}
