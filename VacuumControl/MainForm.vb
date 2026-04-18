Imports System
Imports System.Drawing
Imports System.Globalization
Imports System.Windows.Forms


    Public Class MainForm
        Inherits Form

        Private _hw As IVacuumHardware
        Private _seq As PumpSequencer
        Private ReadOnly _pollTimer As Timer = New Timer()

        ' Pressure readouts
        Private lblIGVal As Label
        Private lblPGVal As Label

        ' Turbo readouts
        Private lblTurboSpeedVal As Label
        Private lblTurboCurrentVal As Label
        Private lblTurboStateVal As Label

        ' Status boxes
        Private lblV1 As Label
        Private lblV2 As Label
        Private lblV3 As Label
        Private lblBackingPump As Label

        ' Automation
        Private lblState As Label
        Private btnPumpdown As Button
        Private btnVent As Button
        Private btnAbort As Button
        Private txtCrossover As TextBox

        ' Log
        Private txtLog As TextBox

        ' Connection panel
        Private cbDemo As CheckBox
        Private txtTpgPort As TextBox
        Private txtTcPort As TextBox
        Private btnConnect As Button
        Private lblConnStatus As Label

        Public Sub New()
            Me.Text = "Vacuum control — HiPace 1200 / Duo 35"
            Me.ClientSize = New Size(870, 650)
            Me.StartPosition = FormStartPosition.CenterScreen
            Me.Font = New Font("Segoe UI", 9.0F)
            Me.MinimumSize = New Size(886, 689)

            BuildUi()
            EnterDemoMode()

            _pollTimer.Interval = 500
            AddHandler _pollTimer.Tick, AddressOf OnPollTick
            _pollTimer.Start()

            AddHandler Me.FormClosing, AddressOf OnFormClosing
        End Sub

        '--------------------------------------------------------
        ' UI construction
        '--------------------------------------------------------

        Private Sub BuildUi()
            ' ---- Pressures ----
            Dim gbP = MakeGroup("Pressures", 10, 10, 420, 90)
            gbP.Controls.Add(MakeLabel("Ion gauge (IG):", 15, 28, 130))
            lblIGVal = MakeMonoLabel("—", 150, 28, 250)
            gbP.Controls.Add(lblIGVal)
            gbP.Controls.Add(MakeLabel("Pirani (PG):", 15, 58, 130))
            lblPGVal = MakeMonoLabel("—", 150, 58, 250)
            gbP.Controls.Add(lblPGVal)
            Controls.Add(gbP)

            ' ---- Turbo ----
            Dim gbT = MakeGroup("HiPace 1200 turbopump", 440, 10, 420, 130)
            gbT.Controls.Add(MakeLabel("Speed:", 15, 28, 110))
            lblTurboSpeedVal = MakeMonoLabel("—", 130, 28, 270)
            gbT.Controls.Add(lblTurboSpeedVal)
            gbT.Controls.Add(MakeLabel("Current:", 15, 58, 110))
            lblTurboCurrentVal = MakeMonoLabel("—", 130, 58, 270)
            gbT.Controls.Add(lblTurboCurrentVal)
            gbT.Controls.Add(MakeLabel("Status:", 15, 88, 110))
            lblTurboStateVal = MakeMonoLabel("—", 130, 88, 270)
            gbT.Controls.Add(lblTurboStateVal)
            Controls.Add(gbT)

            ' ---- Valves ----
            Dim gbV = MakeGroup("Valves", 10, 110, 420, 130)
            gbV.Controls.Add(MakeLabel("V1 roughing:", 15, 28, 120))
            lblV1 = MakeStatusBox("CLOSED", 140, 25)
            gbV.Controls.Add(lblV1)
            gbV.Controls.Add(MakeLabel("V2 foreline:", 15, 58, 120))
            lblV2 = MakeStatusBox("CLOSED", 140, 55)
            gbV.Controls.Add(lblV2)
            gbV.Controls.Add(MakeLabel("V3 vent:", 15, 88, 120))
            lblV3 = MakeStatusBox("CLOSED", 140, 85)
            gbV.Controls.Add(lblV3)
            Controls.Add(gbV)

            ' ---- Backing pump ----
            Dim gbB = MakeGroup("Duo 35 backing pump", 440, 150, 420, 90)
            gbB.Controls.Add(MakeLabel("Status:", 15, 33, 110))
            lblBackingPump = MakeStatusBox("OFF", 130, 30)
            gbB.Controls.Add(lblBackingPump)
            Controls.Add(gbB)

            ' ---- Automation ----
            Dim gbA = MakeGroup("Automation", 10, 250, 850, 130)
            gbA.Controls.Add(MakeLabel("State:", 15, 28, 70))
            lblState = New Label With {
                .Text = "Idle",
                .Location = New Point(90, 28),
                .Size = New Size(400, 22),
                .Font = New Font("Segoe UI", 10.0F, FontStyle.Bold),
                .ForeColor = Color.DarkSlateBlue
            }
            gbA.Controls.Add(lblState)

            btnPumpdown = New Button With {
                .Text = "Pump down",
                .Location = New Point(15, 60),
                .Size = New Size(150, 42),
                .BackColor = Color.FromArgb(200, 230, 200),
                .FlatStyle = FlatStyle.System
            }
            btnVent = New Button With {
                .Text = "Vent",
                .Location = New Point(175, 60),
                .Size = New Size(150, 42),
                .BackColor = Color.FromArgb(240, 220, 180),
                .FlatStyle = FlatStyle.System
            }
            btnAbort = New Button With {
                .Text = "Abort / safe state",
                .Location = New Point(335, 60),
                .Size = New Size(150, 42),
                .BackColor = Color.FromArgb(240, 200, 200),
                .FlatStyle = FlatStyle.System
            }

            AddHandler btnPumpdown.Click, AddressOf OnPumpdownClick
            AddHandler btnVent.Click, AddressOf OnVentClick
            AddHandler btnAbort.Click, AddressOf OnAbortClick

            gbA.Controls.Add(btnPumpdown)
            gbA.Controls.Add(btnVent)
            gbA.Controls.Add(btnAbort)

            gbA.Controls.Add(MakeLabel("Crossover pressure:", 520, 70, 140))
            txtCrossover = New TextBox With {
                .Text = "0.1",
                .Location = New Point(665, 67),
                .Size = New Size(70, 22)
            }
            gbA.Controls.Add(txtCrossover)
            gbA.Controls.Add(MakeLabel("mbar", 740, 70, 60))

            Controls.Add(gbA)

            ' ---- Log ----
            Dim gbL = MakeGroup("Log", 10, 390, 850, 170)
            txtLog = New TextBox With {
                .Multiline = True,
                .ScrollBars = ScrollBars.Vertical,
                .ReadOnly = True,
                .Location = New Point(10, 22),
                .Size = New Size(830, 140),
                .Font = New Font("Consolas", 9.0F),
                .BackColor = Color.FromArgb(250, 250, 248)
            }
            gbL.Controls.Add(txtLog)
            Controls.Add(gbL)

            ' ---- Hardware connection ----
            Dim gbC = MakeGroup("Hardware", 10, 570, 850, 70)
            cbDemo = New CheckBox With {
                .Text = "Demo mode (no hardware)",
                .Location = New Point(15, 30),
                .Size = New Size(180, 22),
                .Checked = True
            }
            AddHandler cbDemo.CheckedChanged, AddressOf OnDemoToggle
            gbC.Controls.Add(cbDemo)

            gbC.Controls.Add(MakeLabel("TPG 366:", 210, 30, 70))
            txtTpgPort = New TextBox With {
                .Text = "COM3",
                .Location = New Point(285, 27),
                .Size = New Size(70, 22)
            }
            gbC.Controls.Add(txtTpgPort)
            gbC.Controls.Add(MakeLabel("TC 1200:", 370, 30, 70))
            txtTcPort = New TextBox With {
                .Text = "COM4",
                .Location = New Point(445, 27),
                .Size = New Size(70, 22)
            }
            gbC.Controls.Add(txtTcPort)

            btnConnect = New Button With {
                .Text = "Connect",
                .Location = New Point(525, 25),
                .Size = New Size(90, 28)
            }
            AddHandler btnConnect.Click, AddressOf OnConnectClick
            gbC.Controls.Add(btnConnect)

            lblConnStatus = New Label With {
                .Text = "Demo mode",
                .Location = New Point(625, 30),
                .Size = New Size(220, 22),
                .ForeColor = Color.DarkOrange,
                .Font = New Font("Segoe UI", 9.0F, FontStyle.Bold)
            }
            gbC.Controls.Add(lblConnStatus)

            Controls.Add(gbC)
        End Sub

        Private Function MakeGroup(text As String, x As Integer, y As Integer, w As Integer, h As Integer) As GroupBox
            Return New GroupBox With {
                .Text = text,
                .Location = New Point(x, y),
                .Size = New Size(w, h)
            }
        End Function

        Private Function MakeLabel(text As String, x As Integer, y As Integer, w As Integer) As Label
            Return New Label With {
                .Text = text,
                .Location = New Point(x, y),
                .Size = New Size(w, 22)
            }
        End Function

        Private Function MakeMonoLabel(text As String, x As Integer, y As Integer, w As Integer) As Label
            Return New Label With {
                .Text = text,
                .Location = New Point(x, y),
                .Size = New Size(w, 22),
                .Font = New Font("Consolas", 11.0F, FontStyle.Bold)
            }
        End Function

        Private Function MakeStatusBox(text As String, x As Integer, y As Integer) As Label
            Return New Label With {
                .Text = text,
                .Location = New Point(x, y),
                .Size = New Size(110, 26),
                .TextAlign = ContentAlignment.MiddleCenter,
                .BorderStyle = BorderStyle.FixedSingle,
                .BackColor = Color.LightGray,
                .Font = New Font("Segoe UI", 9.0F, FontStyle.Bold),
                .ForeColor = Color.DimGray
            }
        End Function

        '--------------------------------------------------------
        ' Mode switching
        '--------------------------------------------------------

        Private Sub EnterDemoMode()
            ReplaceHardware(New DemoHardware())
            lblConnStatus.Text = "Demo mode active"
            lblConnStatus.ForeColor = Color.DarkOrange
            AppendLog("Demo mode active (no hardware required).")
            _hw.SetBackingPumpOn(True)
        End Sub

        Private Sub OnDemoToggle(sender As Object, e As EventArgs)
            If cbDemo.Checked Then EnterDemoMode()
        End Sub

        Private Sub OnConnectClick(sender As Object, e As EventArgs)
            If cbDemo.Checked Then
                EnterDemoMode()
                Return
            End If
            Try
                Dim real = New RealHardware(txtTpgPort.Text.Trim(), txtTcPort.Text.Trim())
                ReplaceHardware(real)
                lblConnStatus.Text = "Connected: TPG 366 + TC 1200"
                lblConnStatus.ForeColor = Color.DarkGreen
                AppendLog("Hardware connected on " & txtTpgPort.Text & " / " & txtTcPort.Text & ".")
            Catch ex As Exception
                AppendLog("Connect failed: " & ex.Message)
                AppendLog("Falling back to demo mode.")
                cbDemo.Checked = True
                EnterDemoMode()
            End Try
        End Sub

        Private Sub ReplaceHardware(newHw As IVacuumHardware)
            If _seq IsNot Nothing Then
                RemoveHandler _seq.StateChanged, AddressOf OnSeqState
                RemoveHandler _seq.LogMessage, AddressOf OnSeqLog
            End If
            Dim oldHw = _hw
            _hw = newHw
            _seq = New PumpSequencer(_hw)
            AddHandler _seq.StateChanged, AddressOf OnSeqState
            AddHandler _seq.LogMessage, AddressOf OnSeqLog

            If TypeOf oldHw Is IDisposable Then
                Try
                    CType(oldHw, IDisposable).Dispose()
                Catch
                End Try
            End If
        End Sub

        '--------------------------------------------------------
        ' Automation button handlers
        '--------------------------------------------------------

        Private Sub OnPumpdownClick(sender As Object, e As EventArgs)
            Dim v As Double
            If Double.TryParse(txtCrossover.Text, NumberStyles.Float, CultureInfo.InvariantCulture, v) AndAlso v > 0 Then
                _seq.CrossoverPressureMbar = v
            End If
            _seq.StartPumpdown()
        End Sub

        Private Sub OnVentClick(sender As Object, e As EventArgs)
            _seq.StartVent()
        End Sub

        Private Sub OnAbortClick(sender As Object, e As EventArgs)
            _seq.Abort()
        End Sub

        '--------------------------------------------------------
        ' Sequencer callbacks (marshal to UI thread)
        '--------------------------------------------------------

        Private Sub OnSeqState(sender As Object, st As SeqState)
            If Me.InvokeRequired Then
                Me.BeginInvoke(New Action(Sub() OnSeqState(sender, st)))
                Return
            End If
            lblState.Text = FormatStateName(st)
        End Sub

        Private Function FormatStateName(st As SeqState) As String
            Select Case st
                Case SeqState.Idle : Return "Idle"
                Case SeqState.PumpDownInit : Return "Pumpdown: initializing"
                Case SeqState.PumpDownRoughing : Return "Pumpdown: roughing (V1 open)"
                Case SeqState.PumpDownCrossover : Return "Pumpdown: crossover"
                Case SeqState.PumpDownTurbo : Return "Pumpdown: turbo accelerating"
                Case SeqState.AtVacuum : Return "At vacuum"
                Case SeqState.VentInit : Return "Vent: stopping turbo"
                Case SeqState.VentTurboSpindown : Return "Vent: turbo spinning down"
                Case SeqState.VentOpening : Return "Vent: N₂ flowing in"
                Case SeqState.VentComplete : Return "Vent complete (atmosphere)"
                Case SeqState.Aborted : Return "ABORTED"
                Case Else : Return st.ToString()
            End Select
        End Function

        Private Sub OnSeqLog(sender As Object, msg As String)
            AppendLog(msg)
        End Sub

        Private Sub AppendLog(msg As String)
            If Me.InvokeRequired Then
                Me.BeginInvoke(New Action(Sub() AppendLog(msg)))
                Return
            End If
            txtLog.AppendText(DateTime.Now.ToString("HH:mm:ss") & "  " & msg & Environment.NewLine)
        End Sub

        '--------------------------------------------------------
        ' Status polling (UI timer)
        '--------------------------------------------------------

        Private Sub OnPollTick(sender As Object, e As EventArgs)
            If _hw Is Nothing Then Return
            Try
                Dim ig = _hw.ReadIonGauge()
                Dim pg = _hw.ReadPiraniGauge()
                Dim ts = _hw.ReadTurboStatus()

                SetPressureLabel(lblIGVal, ig)
                SetPressureLabel(lblPGVal, pg)

                lblTurboSpeedVal.Text = String.Format("{0} Hz ({1}%)", ts.SpeedHz, ts.SpeedPercent)
                lblTurboCurrentVal.Text = String.Format("{0:0.00} A", ts.CurrentA)
                lblTurboStateVal.Text = ts.State.ToString()

                SetValveLabel(lblV1, _hw.GetValveV1())
                SetValveLabel(lblV2, _hw.GetValveV2())
                SetValveLabel(lblV3, _hw.GetValveV3())
                SetPumpLabel(lblBackingPump, _hw.IsBackingPumpOn())
            Catch
                ' swallow polling errors so the UI stays alive
            End Try
        End Sub

        Private Sub SetPressureLabel(lbl As Label, g As GaugeReading)
            If g.Status = GaugeStatus.Ok Then
                lbl.Text = String.Format(CultureInfo.InvariantCulture, "{0:0.00E+0} mbar", g.PressureMbar)
                lbl.ForeColor = Color.Black
            Else
                lbl.Text = g.Status.ToString()
                lbl.ForeColor = Color.Gray
            End If
        End Sub

        Private Sub SetValveLabel(lbl As Label, openVal As Boolean)
            If openVal Then
                lbl.Text = "OPEN"
                lbl.BackColor = Color.FromArgb(110, 190, 110)
                lbl.ForeColor = Color.White
            Else
                lbl.Text = "CLOSED"
                lbl.BackColor = Color.LightGray
                lbl.ForeColor = Color.DimGray
            End If
        End Sub

        Private Sub SetPumpLabel(lbl As Label, onVal As Boolean)
            If onVal Then
                lbl.Text = "ON"
                lbl.BackColor = Color.FromArgb(110, 190, 110)
                lbl.ForeColor = Color.White
            Else
                lbl.Text = "OFF"
                lbl.BackColor = Color.LightGray
                lbl.ForeColor = Color.DimGray
            End If
        End Sub

        '--------------------------------------------------------
        ' Shutdown
        '--------------------------------------------------------

        Private Sub OnFormClosing(sender As Object, e As FormClosingEventArgs)
            _pollTimer.Stop()
            Try
                If _seq IsNot Nothing AndAlso _seq.IsRunning Then _seq.Abort()
            Catch
            End Try
            If TypeOf _hw Is IDisposable Then
                Try
                    CType(_hw, IDisposable).Dispose()
                Catch
                End Try
            End If
        End Sub

    End Class

