Imports System
Imports System.Globalization
Imports System.IO.Ports
Imports System.Text
Imports System.Threading


    '================================================================
    ' Data types
    '================================================================

    Public Enum GaugeStatus
        Ok = 0
        Underrange = 1
        Overrange = 2
        SensorError = 3
        SensorOff = 4
        NoSensor = 5
        IdentificationError = 6
        Disconnected = 9
    End Enum

    Public Class GaugeReading
        Public Property Status As GaugeStatus
        Public Property PressureMbar As Double
    End Class

    Public Enum TurboState
        Stopped
        Accelerating
        AtSpeed
        Decelerating
        Faulted
        Disconnected
    End Enum

    Public Class TurboStatus
        Public Property State As TurboState
        Public Property SpeedHz As Integer
        Public Property NominalSpeedHz As Integer = 630  ' HiPace 1200 default
        Public Property CurrentA As Double
        Public Property ErrorCode As String = ""

        Public ReadOnly Property SpeedPercent As Integer
            Get
                If NominalSpeedHz = 0 Then Return 0
                Return CInt(100.0 * SpeedHz / NominalSpeedHz)
            End Get
        End Property
    End Class

    '================================================================
    ' Interface — both real hardware and demo implement this
    '================================================================

    Public Interface IVacuumHardware
        Function IsConnected() As Boolean
        Function ReadIonGauge() As GaugeReading
        Function ReadPiraniGauge() As GaugeReading
        Function ReadTurboStatus() As TurboStatus
        Sub SetTurboOn(onOff As Boolean)
        Sub SetBackingPumpOn(onOff As Boolean)
        Function IsBackingPumpOn() As Boolean
        Sub SetValveV1(openVal As Boolean)
        Sub SetValveV2(openVal As Boolean)
        Sub SetValveV3(openVal As Boolean)
        Function GetValveV1() As Boolean
        Function GetValveV2() As Boolean
        Function GetValveV3() As Boolean
        ReadOnly Property IsDemo As Boolean
    End Interface

    '================================================================
    ' TPG 366 MaxiGauge driver — ASCII mnemonic protocol
    '   See Pfeiffer Vacuum protocol BG5511BEN (TPG 366 Communication
    '   Protocol) for full command set.
    '
    '   Read channel: send "PRx<CR><LF>", wait for <ACK><CR><LF>,
    '   send <ENQ>, read "status,pressure<CR><LF>".
    '================================================================

    Public Class Tpg366Driver
        Implements IDisposable

        Private Shared ReadOnly AckStr As String = ChrW(6).ToString()
        Private Shared ReadOnly EnqStr As String = ChrW(5).ToString()

        Private ReadOnly _port As SerialPort
        Private ReadOnly _lock As New Object()

        Public Sub New(portName As String)
            _port = New SerialPort(portName, 9600, Parity.None, 8, StopBits.One)
            _port.NewLine = Chr(13) & Chr(10)
            _port.ReadTimeout = 800
            _port.WriteTimeout = 800
            _port.Handshake = Handshake.None
            _port.Open()
        End Sub

        Public Function ReadChannel(channel As Integer) As GaugeReading
            SyncLock _lock
                Try
                    _port.DiscardInBuffer()
                    _port.Write("PR" & channel.ToString() & Chr(13) & Chr(10))

                    Dim ackLine = _port.ReadLine()
                    If String.IsNullOrEmpty(ackLine) OrElse Not ackLine.Contains(AckStr) Then
                        Return New GaugeReading With {.Status = GaugeStatus.Disconnected}
                    End If

                    _port.Write(EnqStr)

                    Dim data = _port.ReadLine()
                    Return ParseMeasurement(data)

                Catch ex As TimeoutException
                    Return New GaugeReading With {.Status = GaugeStatus.Disconnected}
                Catch ex As Exception
                    Return New GaugeReading With {.Status = GaugeStatus.Disconnected}
                End Try
            End SyncLock
        End Function

        Private Function ParseMeasurement(raw As String) As GaugeReading
            If String.IsNullOrWhiteSpace(raw) Then
                Return New GaugeReading With {.Status = GaugeStatus.Disconnected}
            End If
            Dim parts = raw.Split(","c)
            If parts.Length < 2 Then
                Return New GaugeReading With {.Status = GaugeStatus.SensorError}
            End If

            Dim statusCode As Integer
            If Not Integer.TryParse(parts(0).Trim(), statusCode) Then
                Return New GaugeReading With {.Status = GaugeStatus.SensorError}
            End If

            Dim pressure As Double = 0.0
            Double.TryParse(parts(1).Trim(), NumberStyles.Float, CultureInfo.InvariantCulture, pressure)

            Return New GaugeReading With {
                .Status = CType(statusCode, GaugeStatus),
                .PressureMbar = pressure
            }
        End Function

        ' Drive one of the 6 TPG 366 relay outputs. The relay number is 1..6.
        ' The TPG 366 also supports pressure-dependent switching — configure
        ' on the front panel or via the SP1..SP6 mnemonics.
        Public Function SetRelay(relayNumber As Integer, closedVal As Boolean) As Boolean
            SyncLock _lock
                Try
                    ' SPS = set switching state manually. Consult the
                    ' TPG 366 protocol doc for exact mnemonic on your firmware.
                    _port.DiscardInBuffer()
                    Dim cmd = String.Format("SPS,{0},{1}{2}{3}",
                                            relayNumber,
                                            If(closedVal, "1", "0"),
                                            Chr(13), Chr(10))
                    _port.Write(cmd)
                    Dim ackLine = _port.ReadLine()
                    Return ackLine IsNot Nothing AndAlso ackLine.Contains(AckStr)
                Catch
                    Return False
                End Try
            End SyncLock
        End Function

        Public Sub Dispose() Implements IDisposable.Dispose
            Try
                If _port IsNot Nothing AndAlso _port.IsOpen Then _port.Close()
                If _port IsNot Nothing Then _port.Dispose()
            Catch
            End Try
        End Sub

    End Class

    '================================================================
    ' TC 1200 turbopump drive driver — Pfeiffer Vacuum Protocol
    '   Telegram:  addr(3) action(2) param(3) len(2) data(n) cksum(3) <CR>
    '   Action "00" = read,  "10" = write
    '   Checksum = sum of ASCII codes of all prior chars, mod 256, 3 digits
    '
    '   Consult TC 1200 manual (PT0526BEN or similar) for full parameter
    '   list. A few common ones used below:
    '
    '     P:010  Pumping station    bool_new (000000/111111)   (on/off)
    '     P:012  Enable vent valve  bool_new
    '     P:023  Motor pump         bool_new
    '     P:309  ActualSpeed        u_integer (Hz)
    '     P:310  DrvCurrent         u_integer (0.01 A)
    '     P:315  NominalSpeed       u_integer (Hz)
    '     P:346  ErrorCode          string_16
    '
    '   Valves wired through the TC 1200 accessory outputs are set via
    '   their configured accessory parameters (see TC 1200 manual, section
    '   "Configuring the accessory outputs").
    '================================================================

    Public Class Tc1200Driver
        Implements IDisposable

        Private ReadOnly _port As SerialPort
        Private ReadOnly _address As Integer
        Private ReadOnly _lock As New Object()

        Public Sub New(portName As String, Optional address As Integer = 1)
            _address = address
            _port = New SerialPort(portName, 9600, Parity.None, 8, StopBits.One)
            _port.NewLine = Chr(13)
            _port.ReadTimeout = 800
            _port.WriteTimeout = 800
            _port.Handshake = Handshake.None
            _port.Open()
        End Sub

        Public Function ReadParameter(paramNumber As Integer) As String
            SyncLock _lock
                Try
                    Dim telegram = BuildTelegram("00", paramNumber, "=?")
                    _port.DiscardInBuffer()
                    _port.Write(telegram)
                    Dim resp = _port.ReadLine()
                    Return ParseResponse(resp)
                Catch
                    Return Nothing
                End Try
            End SyncLock
        End Function

        Public Function WriteParameter(paramNumber As Integer, data As String) As Boolean
            SyncLock _lock
                Try
                    Dim telegram = BuildTelegram("10", paramNumber, data)
                    _port.DiscardInBuffer()
                    _port.Write(telegram)
                    Dim resp = _port.ReadLine()
                    Return resp IsNot Nothing
                Catch
                    Return False
                End Try
            End SyncLock
        End Function

        Private Function BuildTelegram(action As String, paramNumber As Integer, data As String) As String
            Dim body = String.Format("{0:D3}{1}{2:D3}{3:D2}{4}",
                                     _address, action, paramNumber, data.Length, data)
            Return body & Checksum(body) & Chr(13)
        End Function

        Private Function Checksum(s As String) As String
            Dim total As Integer = 0
            For i = 0 To s.Length - 1
                total += AscW(s(i))
            Next
            Return (total Mod 256).ToString("D3")
        End Function

        Private Function ParseResponse(response As String) As String
            If String.IsNullOrEmpty(response) OrElse response.Length < 13 Then Return Nothing
            Dim lenStr = response.Substring(8, 2)
            Dim dataLen As Integer
            If Not Integer.TryParse(lenStr, dataLen) Then Return Nothing
            If response.Length < 10 + dataLen Then Return Nothing
            Return response.Substring(10, dataLen)
        End Function

        ' -- Convenience helpers --

        Public Function SetPumpingStation(onOff As Boolean) As Boolean
            Return WriteParameter(10, If(onOff, "111111", "000000"))
        End Function

        Public Function SetVentValve(openVal As Boolean) As Boolean
            Return WriteParameter(12, If(openVal, "111111", "000000"))
        End Function

        Public Function ReadActualSpeedHz() As Integer
            Return TryParseInt(ReadParameter(309))
        End Function

        Public Function ReadNominalSpeedHz() As Integer
            Dim v = TryParseInt(ReadParameter(315))
            If v = 0 Then Return 630
            Return v
        End Function

        Public Function ReadDriveCurrentA() As Double
            Return TryParseInt(ReadParameter(310)) / 100.0
        End Function

        Public Function ReadErrorCode() As String
            Dim s = ReadParameter(346)
            If s Is Nothing Then Return ""
            Return s.Trim()
        End Function

        Private Shared Function TryParseInt(s As String) As Integer
            If String.IsNullOrEmpty(s) Then Return 0
            Dim v As Integer
            If Integer.TryParse(s.Trim(), v) Then Return v
            Return 0
        End Function

        Public Sub Dispose() Implements IDisposable.Dispose
            Try
                If _port IsNot Nothing AndAlso _port.IsOpen Then _port.Close()
                If _port IsNot Nothing Then _port.Dispose()
            Catch
            End Try
        End Sub

    End Class

    '================================================================
    ' RealHardware — wraps the two drivers
    '
    ' Valve wiring assumptions (adjust to your real setup):
    '   V1 roughing  -> TPG 366 relay 1 (pressure-dependent switching also OK)
    '   V2 foreline  -> TC 1200 accessory A  (or wire to another relay)
    '   V3 vent      -> TC 1200 dedicated vent valve (P:012)
    '   Duo 35 power -> TC 1200 backing pump relay (via Pfeiffer relay box)
    '================================================================

    Public Class RealHardware
        Implements IVacuumHardware, IDisposable

        Private ReadOnly _tpg As Tpg366Driver
        Private ReadOnly _tc As Tc1200Driver

        Private _v1Open As Boolean
        Private _v2Open As Boolean
        Private _v3Open As Boolean
        Private _turboCmd As Boolean
        Private _turboRampStart As DateTime
        Private _backingOn As Boolean

        Public Sub New(tpgPort As String, tcPort As String)
            _tpg = New Tpg366Driver(tpgPort)
            Try
                _tc = New Tc1200Driver(tcPort)
            Catch
                _tpg.Dispose()
                Throw
            End Try
        End Sub

        Public ReadOnly Property IsDemo As Boolean Implements IVacuumHardware.IsDemo
            Get
                Return False
            End Get
        End Property

        Public Function IsConnected() As Boolean Implements IVacuumHardware.IsConnected
            Return _tpg IsNot Nothing AndAlso _tc IsNot Nothing
        End Function

        Public Function ReadIonGauge() As GaugeReading Implements IVacuumHardware.ReadIonGauge
            Return _tpg.ReadChannel(1)
        End Function

        Public Function ReadPiraniGauge() As GaugeReading Implements IVacuumHardware.ReadPiraniGauge
            Return _tpg.ReadChannel(2)
        End Function

        Public Function ReadTurboStatus() As TurboStatus Implements IVacuumHardware.ReadTurboStatus
            Dim ts As New TurboStatus()
            ts.SpeedHz = _tc.ReadActualSpeedHz()
            ts.NominalSpeedHz = _tc.ReadNominalSpeedHz()
            ts.CurrentA = _tc.ReadDriveCurrentA()
            ts.ErrorCode = _tc.ReadErrorCode()

            Dim pct = ts.SpeedPercent
            If Not String.IsNullOrEmpty(ts.ErrorCode) AndAlso ts.ErrorCode <> "000000" Then
                ts.State = TurboState.Faulted
            ElseIf pct >= 95 Then
                ts.State = TurboState.AtSpeed
            ElseIf pct < 5 AndAlso Not _turboCmd Then
                ts.State = TurboState.Stopped
            ElseIf _turboCmd Then
                ts.State = TurboState.Accelerating
            Else
                ts.State = TurboState.Decelerating
            End If
            Return ts
        End Function

        Public Sub SetTurboOn(onOff As Boolean) Implements IVacuumHardware.SetTurboOn
            _turboCmd = onOff
            _turboRampStart = DateTime.Now
            _tc.SetPumpingStation(onOff)
        End Sub

        Public Sub SetValveV1(openVal As Boolean) Implements IVacuumHardware.SetValveV1
            _v1Open = openVal
            _tpg.SetRelay(1, openVal)
        End Sub

        Public Sub SetValveV2(openVal As Boolean) Implements IVacuumHardware.SetValveV2
            _v2Open = openVal
            ' Wire V2 to a TC 1200 accessory output in your configuration, then
            ' drive the configured parameter here. Placeholder — see TC 1200 manual:
            _tc.WriteParameter(788, If(openVal, "111111", "000000"))
        End Sub

        Public Sub SetValveV3(openVal As Boolean) Implements IVacuumHardware.SetValveV3
            _v3Open = openVal
            _tc.SetVentValve(openVal)
        End Sub

        Public Function GetValveV1() As Boolean Implements IVacuumHardware.GetValveV1
            Return _v1Open
        End Function
        Public Function GetValveV2() As Boolean Implements IVacuumHardware.GetValveV2
            Return _v2Open
        End Function
        Public Function GetValveV3() As Boolean Implements IVacuumHardware.GetValveV3
            Return _v3Open
        End Function

        Public Sub SetBackingPumpOn(onOff As Boolean) Implements IVacuumHardware.SetBackingPumpOn
            _backingOn = onOff
            ' Backing pump is typically wired through a TC 1200 relay (parameter
            ' 025 or via a configured accessory output). Placeholder:
            _tc.WriteParameter(25, If(onOff, "111111", "000000"))
        End Sub

        Public Function IsBackingPumpOn() As Boolean Implements IVacuumHardware.IsBackingPumpOn
            Return _backingOn
        End Function

        Public Sub Dispose() Implements IDisposable.Dispose
            Try
                _tpg?.Dispose()
            Catch
            End Try
            Try
                _tc?.Dispose()
            Catch
            End Try
        End Sub

    End Class

    '================================================================
    ' DemoHardware — physically plausible simulator
    '
    ' Pressure dynamics: exponential approach in log10(p) toward a target
    ' pressure that depends on valve/pump state. Turbo speed ramps up/down.
    '================================================================

    Public Class DemoHardware
        Implements IVacuumHardware

        Private _v1Open As Boolean
        Private _v2Open As Boolean
        Private _v3Open As Boolean
        Private _turboCmd As Boolean
        Private _backingOn As Boolean

        Private _chamberMbar As Double = 1013.0
        Private _forelineMbar As Double = 1013.0
        Private _turboSpeedHz As Double = 0.0
        Private Const NominalHz As Integer = 630

        Private _lastUpdate As DateTime = DateTime.Now
        Private ReadOnly _rng As New Random()

        Private Sub Update()
            Dim now = DateTime.Now
            Dim dt = (now - _lastUpdate).TotalSeconds
            _lastUpdate = now
            If dt <= 0 Then Return
            If dt > 1.0 Then dt = 1.0

            ' --- Chamber pressure target ---
            Dim chamberTarget As Double = _chamberMbar
            Dim chamberTau As Double = 120.0

            If _v3Open Then
                chamberTarget = 1013.0
                chamberTau = 8.0
            ElseIf _v1Open AndAlso _backingOn Then
                chamberTarget = 5.0E-2
                chamberTau = 20.0
            ElseIf _v2Open AndAlso _backingOn AndAlso Not _v1Open AndAlso
                   _turboSpeedHz > 0.9 * NominalHz Then
                chamberTarget = 1.0E-8
                chamberTau = 60.0
            Else
                ' isolated — very slow rise from outgassing
                chamberTarget = Math.Min(1013.0, Math.Max(_chamberMbar * 1.0005, 1.0E-9))
                chamberTau = 500.0
            End If
            _chamberMbar = ExpApproachLog(_chamberMbar, chamberTarget, dt, chamberTau)

            ' --- Foreline pressure ---
            Dim foreTarget As Double
            Dim foreTau As Double = 5.0
            If _backingOn Then
                If _v2Open AndAlso _turboSpeedHz > 0.5 * NominalHz Then
                    foreTarget = 0.8
                Else
                    foreTarget = 2.0E-2
                End If
            Else
                foreTarget = 1013.0
                foreTau = 25.0
            End If
            _forelineMbar = ExpApproachLog(_forelineMbar, foreTarget, dt, foreTau)

            ' --- Turbo speed ---
            Dim speedTarget As Double
            If _turboCmd AndAlso _forelineMbar < 5.0 Then
                speedTarget = NominalHz
                _turboSpeedHz += (speedTarget - _turboSpeedHz) * (dt / 12.0)
            Else
                speedTarget = 0
                _turboSpeedHz += (speedTarget - _turboSpeedHz) * (dt / 25.0)
            End If
            If _turboSpeedHz < 0.5 Then _turboSpeedHz = 0
        End Sub

        Private Shared Function ExpApproachLog(current As Double, target As Double, dt As Double, tau As Double) As Double
            Dim cur = Math.Max(current, 1.0E-12)
            Dim tgt = Math.Max(target, 1.0E-12)
            Dim logCur = Math.Log10(cur)
            Dim logTgt = Math.Log10(tgt)
            Dim alpha = 1.0 - Math.Exp(-dt / tau)
            Dim logNew = logCur + (logTgt - logCur) * alpha
            Return Math.Pow(10.0, logNew)
        End Function

        Public ReadOnly Property IsDemo As Boolean Implements IVacuumHardware.IsDemo
            Get
                Return True
            End Get
        End Property

        Public Function IsConnected() As Boolean Implements IVacuumHardware.IsConnected
            Return True
        End Function

        Public Function ReadIonGauge() As GaugeReading Implements IVacuumHardware.ReadIonGauge
            Update()
            If _chamberMbar > 1.0E-2 Then
                Return New GaugeReading With {.Status = GaugeStatus.Overrange, .PressureMbar = 0}
            End If
            Dim noise = 1.0 + (_rng.NextDouble() - 0.5) * 0.04
            Return New GaugeReading With {.Status = GaugeStatus.Ok, .PressureMbar = _chamberMbar * noise}
        End Function

        Public Function ReadPiraniGauge() As GaugeReading Implements IVacuumHardware.ReadPiraniGauge
            Update()
            If _chamberMbar < 1.0E-4 Then
                Return New GaugeReading With {.Status = GaugeStatus.Underrange, .PressureMbar = 0}
            End If
            Dim noise = 1.0 + (_rng.NextDouble() - 0.5) * 0.04
            Return New GaugeReading With {.Status = GaugeStatus.Ok, .PressureMbar = _chamberMbar * noise}
        End Function

        Public Function ReadTurboStatus() As TurboStatus Implements IVacuumHardware.ReadTurboStatus
            Update()
            Dim ts As New TurboStatus With {
                .SpeedHz = CInt(_turboSpeedHz),
                .NominalSpeedHz = NominalHz,
                .CurrentA = If(_turboCmd, 0.4 + 0.5 * (_turboSpeedHz / NominalHz), 0.05 * (_turboSpeedHz / NominalHz))
            }
            Dim pct = ts.SpeedPercent
            If Not _turboCmd AndAlso pct < 3 Then
                ts.State = TurboState.Stopped
            ElseIf pct >= 95 Then
                ts.State = TurboState.AtSpeed
            ElseIf _turboCmd Then
                ts.State = TurboState.Accelerating
            Else
                ts.State = TurboState.Decelerating
            End If
            Return ts
        End Function

        Public Sub SetTurboOn(onOff As Boolean) Implements IVacuumHardware.SetTurboOn
            _turboCmd = onOff
        End Sub

        Public Sub SetValveV1(openVal As Boolean) Implements IVacuumHardware.SetValveV1
            _v1Open = openVal
        End Sub

        Public Sub SetValveV2(openVal As Boolean) Implements IVacuumHardware.SetValveV2
            _v2Open = openVal
        End Sub

        Public Sub SetValveV3(openVal As Boolean) Implements IVacuumHardware.SetValveV3
            _v3Open = openVal
        End Sub

        Public Function GetValveV1() As Boolean Implements IVacuumHardware.GetValveV1
            Return _v1Open
        End Function
        Public Function GetValveV2() As Boolean Implements IVacuumHardware.GetValveV2
            Return _v2Open
        End Function
        Public Function GetValveV3() As Boolean Implements IVacuumHardware.GetValveV3
            Return _v3Open
        End Function

        Public Sub SetBackingPumpOn(onOff As Boolean) Implements IVacuumHardware.SetBackingPumpOn
            _backingOn = onOff
        End Sub

        Public Function IsBackingPumpOn() As Boolean Implements IVacuumHardware.IsBackingPumpOn
            Return _backingOn
        End Function

    End Class

