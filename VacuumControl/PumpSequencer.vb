Imports System
Imports System.Diagnostics
Imports System.Threading
Imports System.Threading.Tasks


    Public Enum SeqState
        Idle
        PumpDownInit
        PumpDownRoughing
        PumpDownCrossover
        PumpDownTurbo
        AtVacuum
        VentInit
        VentTurboSpindown
        VentOpening
        VentComplete
        Aborted
    End Enum

    '================================================================
    ' PumpSequencer — runs pumpdown and vent sequences on a background
    ' task, raising StateChanged and LogMessage events as it progresses.
    '================================================================

    Public Class PumpSequencer

        Private ReadOnly _hw As IVacuumHardware
        Private _state As SeqState = SeqState.Idle
        Private _cts As CancellationTokenSource
        Private _task As Task
        Private ReadOnly _lock As New Object()

        ' Tunable thresholds
        Public Property CrossoverPressureMbar As Double = 0.1
        Public Property AtVacuumPressureMbar As Double = 1.0E-5
        Public Property TurboSpindownThresholdPct As Integer = 10
        Public Property VentCompletePressureMbar As Double = 800.0

        Public Event StateChanged(sender As Object, state As SeqState)
        Public Event LogMessage(sender As Object, msg As String)

        Public Sub New(hw As IVacuumHardware)
            _hw = hw
        End Sub

        Public ReadOnly Property State As SeqState
            Get
                Return _state
            End Get
        End Property

        Public ReadOnly Property IsRunning As Boolean
            Get
                Return _task IsNot Nothing AndAlso Not _task.IsCompleted
            End Get
        End Property

        Public Sub StartPumpdown()
            SyncLock _lock
                If IsRunning Then Return
                _cts = New CancellationTokenSource()
                Dim ct = _cts.Token
                _task = Task.Run(Sub() RunPumpdown(ct))
            End SyncLock
        End Sub

        Public Sub StartVent()
            SyncLock _lock
                If IsRunning Then Return
                _cts = New CancellationTokenSource()
                Dim ct = _cts.Token
                _task = Task.Run(Sub() RunVent(ct))
            End SyncLock
        End Sub

        Public Sub Abort()
            SyncLock _lock
                If _cts IsNot Nothing Then
                    Try
                        _cts.Cancel()
                    Catch
                    End Try
                End If
            End SyncLock

            ' Force safe state regardless of what was running
            Try
                _hw.SetTurboOn(False)
            Catch
            End Try
            Try
                _hw.SetValveV1(False)
            Catch
            End Try
            Try
                _hw.SetValveV2(False)
            Catch
            End Try
            Try
                _hw.SetValveV3(False)
            Catch
            End Try

            SetState(SeqState.Aborted)
            Log("Aborted. All valves closed, turbo commanded off, backing pump left running.")
        End Sub

        Private Sub SetState(s As SeqState)
            _state = s
            RaiseEvent StateChanged(Me, s)
        End Sub

        Private Sub Log(msg As String)
            RaiseEvent LogMessage(Me, msg)
        End Sub

        Private Sub Wait(ms As Integer, ct As CancellationToken)
            Dim elapsed As Integer = 0
            Dim step_ As Integer = 100
            While elapsed < ms
                ct.ThrowIfCancellationRequested()
                Thread.Sleep(step_)
                elapsed += step_
            End While
        End Sub

        '------------------------------------------------------------
        ' Pumpdown sequence:
        '   1. Safe start: close V2, V3; turbo off; backing pump on; V1 closed
        '   2. Open V1 (rough chamber via Duo 35)
        '   3. Wait for chamber PG <= crossover pressure
        '   4. Close V1, open V2
        '   5. Command turbo on
        '   6. Wait for turbo AtSpeed
        '   7. Wait for chamber IG <= AtVacuum pressure
        '------------------------------------------------------------
        Private Sub RunPumpdown(ct As CancellationToken)
            Try
                SetState(SeqState.PumpDownInit)
                Log("Pumpdown requested.")

                _hw.SetValveV3(False)
                _hw.SetValveV2(False)
                _hw.SetValveV1(False)
                _hw.SetTurboOn(False)
                _hw.SetBackingPumpOn(True)
                Log("Safe start: all valves closed, backing pump on, turbo off.")
                Wait(1500, ct)

                ' Phase 1: roughing
                Log("Opening V1 (roughing line).")
                _hw.SetValveV1(True)
                SetState(SeqState.PumpDownRoughing)

                Dim roughTimeoutMs = 10 * 60 * 1000 ' 10 min absolute cap
                Dim sw = Stopwatch.StartNew()
                Do
                    ct.ThrowIfCancellationRequested()
                    Wait(500, ct)
                    Dim pg = _hw.ReadPiraniGauge()
                    If pg.Status = GaugeStatus.Ok AndAlso pg.PressureMbar <= CrossoverPressureMbar Then
                        Log(String.Format("Crossover reached at {0:0.00E+0} mbar.", pg.PressureMbar))
                        Exit Do
                    End If
                    If sw.ElapsedMilliseconds > roughTimeoutMs Then
                        Throw New TimeoutException("Roughing timed out — check for leaks or pump fault.")
                    End If
                Loop

                ' Phase 2: crossover to turbo
                SetState(SeqState.PumpDownCrossover)
                Log("Closing V1.")
                _hw.SetValveV1(False)
                Wait(1000, ct)
                Log("Opening V2 (foreline).")
                _hw.SetValveV2(True)
                Wait(500, ct)

                ' Phase 3: spin up turbo
                SetState(SeqState.PumpDownTurbo)
                Log("Commanding turbo on.")
                _hw.SetTurboOn(True)

                Dim turboTimeoutMs = 15 * 60 * 1000
                sw.Restart()
                Do
                    ct.ThrowIfCancellationRequested()
                    Wait(1000, ct)
                    Dim ts = _hw.ReadTurboStatus()
                    If ts.State = TurboState.Faulted Then
                        Throw New InvalidOperationException("Turbo reported a fault: " & ts.ErrorCode)
                    End If
                    If ts.State = TurboState.AtSpeed Then
                        Log(String.Format("Turbo at speed ({0} Hz).", ts.SpeedHz))
                        Exit Do
                    End If
                    If sw.ElapsedMilliseconds > turboTimeoutMs Then
                        Throw New TimeoutException("Turbo did not reach nominal speed in 15 min.")
                    End If
                Loop

                ' Wait for high vacuum
                Log("Waiting for chamber to reach high vacuum...")
                Do
                    ct.ThrowIfCancellationRequested()
                    Wait(2000, ct)
                    Dim ig = _hw.ReadIonGauge()
                    If ig.Status = GaugeStatus.Ok AndAlso ig.PressureMbar <= AtVacuumPressureMbar Then
                        Log(String.Format("Reached {0:0.00E+0} mbar.", ig.PressureMbar))
                        Exit Do
                    End If
                Loop

                SetState(SeqState.AtVacuum)
                Log("At vacuum. System ready.")

            Catch oce As OperationCanceledException
                Log("Pumpdown canceled.")
            Catch ex As Exception
                Log("Pumpdown error: " & ex.Message)
                Abort()
            End Try
        End Sub

        '------------------------------------------------------------
        ' Vent sequence:
        '   1. Command turbo off
        '   2. Wait for turbo speed <= spindown threshold
        '   3. Close V2 (isolate backing pump)
        '   4. Open V3 (vent with N2)
        '   5. Wait for chamber to reach near-atmospheric
        '   6. Close V3
        '------------------------------------------------------------
        Private Sub RunVent(ct As CancellationToken)
            Try
                SetState(SeqState.VentInit)
                Log("Vent requested.")

                Log("Commanding turbo off.")
                _hw.SetTurboOn(False)
                SetState(SeqState.VentTurboSpindown)

                Dim spindownTimeoutMs = 30 * 60 * 1000
                Dim sw = Stopwatch.StartNew()
                Do
                    ct.ThrowIfCancellationRequested()
                    Wait(1000, ct)
                    Dim ts = _hw.ReadTurboStatus()
                    If ts.SpeedPercent <= TurboSpindownThresholdPct Then
                        Log(String.Format("Turbo spun down to {0}%.", ts.SpeedPercent))
                        Exit Do
                    End If
                    If sw.ElapsedMilliseconds > spindownTimeoutMs Then
                        Throw New TimeoutException("Turbo did not spin down in 30 min.")
                    End If
                Loop

                Log("Closing V2 (isolating turbo from foreline).")
                _hw.SetValveV2(False)
                Wait(500, ct)

                Log("Opening V3 (N₂ vent).")
                _hw.SetValveV3(True)
                SetState(SeqState.VentOpening)

                Dim ventTimeoutMs = 5 * 60 * 1000
                sw.Restart()
                Do
                    ct.ThrowIfCancellationRequested()
                    Wait(1000, ct)
                    Dim pg = _hw.ReadPiraniGauge()
                    If pg.Status = GaugeStatus.Overrange Then Exit Do
                    If pg.Status = GaugeStatus.Ok AndAlso pg.PressureMbar >= VentCompletePressureMbar Then Exit Do
                    If sw.ElapsedMilliseconds > ventTimeoutMs Then
                        Throw New TimeoutException("Vent timed out — check N₂ supply.")
                    End If
                Loop

                Log("Chamber at atmosphere. Closing V3.")
                _hw.SetValveV3(False)
                SetState(SeqState.VentComplete)

            Catch oce As OperationCanceledException
                Log("Vent canceled.")
            Catch ex As Exception
                Log("Vent error: " & ex.Message)
                Abort()
            End Try
        End Sub

    End Class

